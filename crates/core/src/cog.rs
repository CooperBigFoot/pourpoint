//! Windowed COG reads for remote raster refinement.

use std::cmp::{max, min};
use std::fs::File;
use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};

use bytes::Bytes;
use geo::Rect;
use object_store::path::Path as ObjectPath;
use object_store::{ObjectStore, ObjectStoreExt};
use tempfile::NamedTempFile;
use tiff::decoder::{Decoder, DecodingResult};
use tiff::encoder::{TiffEncoder, colortype};
use tiff::tags::Tag;
use tracing::debug;

#[cfg(feature = "test-fixtures")]
use crate::algo::geo_transform::GeoTransform;
#[cfg(feature = "test-fixtures")]
use crate::algo::projection::NativeCoord;
use crate::error::CacheError;
use crate::session::RasterKind;

const HEADER_RANGE_BYTES: u64 = 16 * 1024 * 1024;
/// Bounded range for D8 declaration extent reads.
///
/// Selecting among 60 D8 declarations may inspect roughly 60 object `head`
/// responses plus 60 small range reads before fetching any selected window.
/// Keeping this at 256 KiB bounds worst-case pre-selection range transfer near
/// 15 MiB and fails loudly when an IFD is too far from the TIFF header.
pub(crate) const EXTENT_HEADER_RANGE_BYTES: u64 = 256 * 1024;
const MODEL_PIXEL_SCALE_TAG: Tag = Tag::ModelPixelScaleTag;
const MODEL_TIEPOINT_TAG: Tag = Tag::ModelTiepointTag;
const GEO_KEY_DIRECTORY_TAG: Tag = Tag::GeoKeyDirectoryTag;
const GDAL_NODATA_TAG: Tag = Tag::GdalNodata;

/// A raster-native window request.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RasterWindowRequest {
    kind: RasterKind,
    bbox: Rect<f64>,
}

impl RasterWindowRequest {
    /// Create a request for `kind` intersecting `bbox`.
    pub(crate) fn new(kind: RasterKind, bbox: Rect<f64>) -> Self {
        Self { kind, bbox }
    }

    pub(crate) fn kind(&self) -> RasterKind {
        self.kind
    }
}

/// A local GeoTIFF window and the remote bytes used to produce it.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalizedRasterWindow {
    path: PathBuf,
    header_bytes: u64,
    tile_bytes: u64,
    tile_count: usize,
    window_pixels: u64,
}

impl LocalizedRasterWindow {
    pub(crate) fn cached(path: PathBuf) -> Self {
        Self {
            path,
            header_bytes: 0,
            tile_bytes: 0,
            tile_count: 0,
            window_pixels: 0,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn header_bytes(&self) -> u64 {
        self.header_bytes
    }

    pub fn tile_bytes(&self) -> u64 {
        self.tile_bytes
    }

    pub fn tile_count(&self) -> usize {
        self.tile_count
    }

    pub fn window_pixels(&self) -> u64 {
        self.window_pixels
    }
}

/// Supported one-band D8 raster sample layouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CogSampleType {
    U8,
    I8,
    F32,
    I32,
}

/// Metadata needed to plan and materialize a COG window.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CogMetadata {
    width: u32,
    height: u32,
    tile_width: u32,
    tile_height: u32,
    origin_x: f64,
    origin_y: f64,
    pixel_width: f64,
    pixel_height: f64,
    nodata: String,
    sample_type: CogSampleType,
    compression: u16,
    predictor: u16,
    tile_offsets: Vec<u64>,
    tile_byte_counts: Vec<u64>,
}

/// Spatial extent decoded from a GeoTIFF header.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CogExtent {
    rect: Rect<f64>,
}

impl CogExtent {
    pub(crate) fn rect(&self) -> Rect<f64> {
        self.rect
    }
}

impl CogMetadata {
    fn tiles_across(&self) -> u32 {
        self.width.div_ceil(self.tile_width)
    }

    fn tiles_down(&self) -> u32 {
        self.height.div_ceil(self.tile_height)
    }
}

/// Pixel-space raster window, half-open in both dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct RasterPixelWindow {
    col_off: u32,
    row_off: u32,
    width: u32,
    height: u32,
}

impl RasterPixelWindow {
    pub(crate) fn from_bbox(metadata: &CogMetadata, bbox: &Rect<f64>) -> Result<Self, String> {
        if metadata.pixel_width <= 0.0 || metadata.pixel_height >= 0.0 {
            return Err(
                "only north-up rasters with positive x and negative y pixels are supported"
                    .to_string(),
            );
        }

        let min_col =
            ((bbox.min().x - metadata.origin_x) / metadata.pixel_width).floor() as i64 - 1;
        let max_col = ((bbox.max().x - metadata.origin_x) / metadata.pixel_width).ceil() as i64 + 1;
        let min_row =
            ((bbox.max().y - metadata.origin_y) / metadata.pixel_height).floor() as i64 - 1;
        let max_row =
            ((bbox.min().y - metadata.origin_y) / metadata.pixel_height).ceil() as i64 + 1;

        let col_off = min_col.clamp(0, metadata.width as i64) as u32;
        let row_off = min_row.clamp(0, metadata.height as i64) as u32;
        let col_end = max_col.clamp(0, metadata.width as i64) as u32;
        let row_end = max_row.clamp(0, metadata.height as i64) as u32;

        let width = col_end.saturating_sub(col_off);
        let height = row_end.saturating_sub(row_off);
        if width == 0 || height == 0 {
            return Err("requested bbox does not intersect raster extent".to_string());
        }

        Ok(Self {
            col_off,
            row_off,
            width,
            height,
        })
    }

    pub(crate) fn cache_fragment(&self) -> String {
        format!(
            "x{}-y{}-w{}-h{}",
            self.col_off, self.row_off, self.width, self.height
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlannedTile {
    index: u32,
    range: Range<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TilePlan {
    tiles: Vec<PlannedTile>,
}

impl TilePlan {
    pub(crate) fn for_window(metadata: &CogMetadata, window: RasterPixelWindow) -> Self {
        let first_tile_col = window.col_off / metadata.tile_width;
        let last_tile_col = (window.col_off + window.width - 1) / metadata.tile_width;
        let first_tile_row = window.row_off / metadata.tile_height;
        let last_tile_row = (window.row_off + window.height - 1) / metadata.tile_height;
        let tiles_across = metadata.tiles_across();

        let tiles = (first_tile_row..=last_tile_row)
            .flat_map(|tile_row| {
                (first_tile_col..=last_tile_col).map(move |tile_col| {
                    let index = tile_row * tiles_across + tile_col;
                    let offset = metadata.tile_offsets[index as usize];
                    let byte_count = metadata.tile_byte_counts[index as usize];
                    PlannedTile {
                        index,
                        range: offset..offset + byte_count,
                    }
                })
            })
            .collect();

        Self { tiles }
    }

    pub(crate) fn ranges(&self) -> Vec<Range<u64>> {
        self.tiles.iter().map(|tile| tile.range.clone()).collect()
    }

    pub(crate) fn byte_count(&self) -> u64 {
        self.tiles
            .iter()
            .map(|tile| tile.range.end - tile.range.start)
            .sum()
    }
}

/// Header-derived plan for a remote COG window.
#[derive(Debug, Clone)]
pub(crate) struct PreparedCogWindow {
    object_size: u64,
    header_end: u64,
    header: Bytes,
    metadata: CogMetadata,
    window: RasterPixelWindow,
    plan: TilePlan,
}

impl PreparedCogWindow {
    pub(crate) fn cache_fragment(&self) -> String {
        self.window.cache_fragment()
    }
}

/// Read COG metadata and plan the intersecting tile byte ranges.
pub(crate) async fn prepare_window(
    store: &dyn ObjectStore,
    remote_path: &ObjectPath,
    request: &RasterWindowRequest,
) -> Result<PreparedCogWindow, CacheError> {
    let object_meta = store
        .head(remote_path)
        .await
        .map_err(|source| CacheError::ObjectStore {
            path: remote_path.clone(),
            source,
        })?;
    let object_size = object_meta.size as u64;
    let header_end = min(HEADER_RANGE_BYTES, object_size);
    let header = store
        .get_range(remote_path, 0..header_end)
        .await
        .map_err(|source| CacheError::ObjectStore {
            path: remote_path.clone(),
            source,
        })?;

    let reader = RangeBackedTiffReader::new(object_size, vec![(0..header_end, header.clone())]);
    let metadata = read_metadata(reader, remote_path)?;
    validate_merit_layout(&metadata, request.kind(), remote_path)?;
    let window = RasterPixelWindow::from_bbox(&metadata, &request.bbox).map_err(|reason| {
        CacheError::UnsupportedCog {
            path: remote_path.clone(),
            reason,
        }
    })?;
    let plan = TilePlan::for_window(&metadata, window);
    Ok(PreparedCogWindow {
        object_size,
        header_end,
        header,
        metadata,
        window,
        plan,
    })
}

/// Read only the bounded COG header range needed for raster extent selection.
pub(crate) async fn read_remote_extent(
    store: &dyn ObjectStore,
    remote_path: &ObjectPath,
) -> Result<CogExtent, CacheError> {
    let object_meta = store
        .head(remote_path)
        .await
        .map_err(|source| CacheError::ObjectStore {
            path: remote_path.clone(),
            source,
        })?;
    let object_size = object_meta.size as u64;
    let header_end = min(EXTENT_HEADER_RANGE_BYTES, object_size);
    let header = store
        .get_range(remote_path, 0..header_end)
        .await
        .map_err(|source| CacheError::ObjectStore {
            path: remote_path.clone(),
            source,
        })?;
    let reader = RangeBackedTiffReader::new(object_size, vec![(0..header_end, header)]);
    match read_extent(reader, remote_path.as_ref()) {
        Ok(extent) => Ok(extent),
        Err(CacheError::Tiff { .. }) if object_size > header_end => {
            Err(CacheError::UnsupportedCog {
                path: remote_path.clone(),
                reason: format!(
                    "extent header too large for bounded {} byte range",
                    EXTENT_HEADER_RANGE_BYTES
                ),
            })
        }
        Err(source) => Err(source),
    }
}

/// Read only local GeoTIFF header tags needed for raster extent selection.
pub(crate) fn read_local_extent(path: &Path) -> Result<CogExtent, CacheError> {
    let file = File::open(path).map_err(|source| CacheError::Io {
        op: "open",
        path: path.to_path_buf(),
        source,
    })?;
    read_extent(file, &path.display().to_string())
}

fn read_extent<R>(reader: R, path: &str) -> Result<CogExtent, CacheError>
where
    R: Read + Seek,
{
    let mut decoder = Decoder::new(reader).map_err(|source| CacheError::Tiff {
        path: path.to_string(),
        source,
    })?;
    let (width, height) = decoder.dimensions().map_err(|source| CacheError::Tiff {
        path: path.to_string(),
        source,
    })?;
    let scale = decoder
        .get_tag_f64_vec(MODEL_PIXEL_SCALE_TAG)
        .map_err(|source| CacheError::Tiff {
            path: path.to_string(),
            source,
        })?;
    let tiepoint = decoder
        .get_tag_f64_vec(MODEL_TIEPOINT_TAG)
        .map_err(|source| CacheError::Tiff {
            path: path.to_string(),
            source,
        })?;
    if scale.len() < 2 || tiepoint.len() < 6 {
        return Err(CacheError::UnsupportedCog {
            path: ObjectPath::from(path.to_string()),
            reason: "missing GeoTIFF model scale or tiepoint values".to_string(),
        });
    }

    let origin_x = tiepoint[3] - tiepoint[0] * scale[0];
    let origin_y = tiepoint[4] + tiepoint[1] * scale[1];
    let pixel_width = scale[0];
    let pixel_height = -scale[1];
    if pixel_width <= 0.0 || pixel_height >= 0.0 {
        return Err(CacheError::UnsupportedCog {
            path: ObjectPath::from(path.to_string()),
            reason: "only north-up rasters with positive x and negative y pixels are supported"
                .to_string(),
        });
    }
    let min_x = origin_x;
    let max_x = origin_x + f64::from(width) * pixel_width;
    let max_y = origin_y;
    let min_y = origin_y + f64::from(height) * pixel_height;
    Ok(CogExtent {
        rect: Rect::new(
            geo::coord! { x: min_x, y: min_y },
            geo::coord! { x: max_x, y: max_y },
        ),
    })
}

/// Read and materialize a planned remote COG window into `canonical`.
pub(crate) async fn fetch_window_to_path(
    store: &dyn ObjectStore,
    remote_path: &ObjectPath,
    prepared: PreparedCogWindow,
    canonical: &Path,
) -> Result<LocalizedRasterWindow, CacheError> {
    let ranges = prepared.plan.ranges();
    let tile_bytes = store
        .get_ranges(remote_path, &ranges)
        .await
        .map_err(|source| CacheError::ObjectStore {
            path: remote_path.clone(),
            source,
        })?;
    let mut backed_ranges = Vec::with_capacity(ranges.len() + 1);
    backed_ranges.push((0..prepared.header_end, prepared.header));
    backed_ranges.extend(ranges.into_iter().zip(tile_bytes));

    let reader = RangeBackedTiffReader::new(prepared.object_size, backed_ranges);
    let window_data = decode_window(
        reader,
        &prepared.metadata,
        prepared.window,
        &prepared.plan,
        remote_path,
    )?;
    write_window_geotiff(
        canonical,
        &prepared.metadata,
        prepared.window,
        &window_data,
        remote_path,
    )?;

    let stats = LocalizedRasterWindow {
        path: canonical.to_path_buf(),
        header_bytes: prepared.header_end,
        tile_bytes: prepared.plan.byte_count(),
        tile_count: prepared.plan.tiles.len(),
        window_pixels: u64::from(prepared.window.width) * u64::from(prepared.window.height),
    };
    debug!(
        path = %canonical.display(),
        cog_header_bytes = stats.header_bytes,
        cog_tile_bytes = stats.tile_bytes,
        cog_tile_count = stats.tile_count,
        window_pixels = stats.window_pixels,
        "materialized remote COG window"
    );
    Ok(stats)
}

fn read_metadata(
    reader: RangeBackedTiffReader,
    remote_path: &ObjectPath,
) -> Result<CogMetadata, CacheError> {
    let mut decoder = Decoder::new(reader).map_err(|source| CacheError::Tiff {
        path: remote_path.as_ref().to_string(),
        source,
    })?;

    let (width, height) = decoder.dimensions().map_err(|source| CacheError::Tiff {
        path: remote_path.as_ref().to_string(),
        source,
    })?;
    let (tile_width, tile_height) = decoder.chunk_dimensions();
    let color_type = decoder.colortype().map_err(|source| CacheError::Tiff {
        path: remote_path.as_ref().to_string(),
        source,
    })?;
    let sample_formats = decoder
        .find_tag_unsigned_vec::<u16>(Tag::SampleFormat)
        .map_err(|source| CacheError::Tiff {
            path: remote_path.as_ref().to_string(),
            source,
        })?
        .unwrap_or_else(|| vec![1]);
    let sample_type = match (color_type, sample_formats.as_slice()) {
        (tiff::ColorType::Gray(8), [1]) => CogSampleType::U8,
        (tiff::ColorType::Gray(8), [2]) => CogSampleType::I8,
        (tiff::ColorType::Gray(32), [3]) => CogSampleType::F32,
        (tiff::ColorType::Gray(32), [2]) => CogSampleType::I32,
        (other, formats) => {
            return Err(CacheError::UnsupportedCog {
                path: remote_path.clone(),
                reason: format!("unsupported sample layout: {other:?} sample_format={formats:?}"),
            });
        }
    };
    let compression = decoder
        .find_tag_unsigned::<u16>(Tag::Compression)
        .map_err(|source| CacheError::Tiff {
            path: remote_path.as_ref().to_string(),
            source,
        })?
        .unwrap_or(1);
    let predictor = decoder
        .find_tag_unsigned::<u16>(Tag::Predictor)
        .map_err(|source| CacheError::Tiff {
            path: remote_path.as_ref().to_string(),
            source,
        })?
        .unwrap_or(1);

    let scale = decoder
        .get_tag_f64_vec(MODEL_PIXEL_SCALE_TAG)
        .map_err(|source| CacheError::Tiff {
            path: remote_path.as_ref().to_string(),
            source,
        })?;
    let tiepoint = decoder
        .get_tag_f64_vec(MODEL_TIEPOINT_TAG)
        .map_err(|source| CacheError::Tiff {
            path: remote_path.as_ref().to_string(),
            source,
        })?;
    if scale.len() < 2 || tiepoint.len() < 6 {
        return Err(CacheError::UnsupportedCog {
            path: remote_path.clone(),
            reason: "missing GeoTIFF model scale or tiepoint values".to_string(),
        });
    }

    let tile_offsets = decoder
        .find_tag_unsigned_vec::<u64>(Tag::TileOffsets)
        .map_err(|source| CacheError::Tiff {
            path: remote_path.as_ref().to_string(),
            source,
        })?
        .ok_or_else(|| CacheError::UnsupportedCog {
            path: remote_path.clone(),
            reason: "missing TileOffsets tag".to_string(),
        })?;
    let tile_byte_counts = decoder
        .find_tag_unsigned_vec::<u64>(Tag::TileByteCounts)
        .map_err(|source| CacheError::Tiff {
            path: remote_path.as_ref().to_string(),
            source,
        })?
        .ok_or_else(|| CacheError::UnsupportedCog {
            path: remote_path.clone(),
            reason: "missing TileByteCounts tag".to_string(),
        })?;
    let nodata = decoder
        .get_tag_ascii_string(GDAL_NODATA_TAG)
        .unwrap_or_else(|_| match sample_type {
            CogSampleType::U8 => "255".to_string(),
            CogSampleType::I8 => "-1".to_string(),
            CogSampleType::F32 => "-1".to_string(),
            CogSampleType::I32 => "-1".to_string(),
        });

    let origin_x = tiepoint[3] - tiepoint[0] * scale[0];
    let origin_y = tiepoint[4] + tiepoint[1] * scale[1];

    Ok(CogMetadata {
        width,
        height,
        tile_width,
        tile_height,
        origin_x,
        origin_y,
        pixel_width: scale[0],
        pixel_height: -scale[1],
        nodata,
        sample_type,
        compression,
        predictor,
        tile_offsets,
        tile_byte_counts,
    })
}

fn validate_merit_layout(
    metadata: &CogMetadata,
    kind: RasterKind,
    remote_path: &ObjectPath,
) -> Result<(), CacheError> {
    if metadata.tile_width != 512 || metadata.tile_height != 512 {
        return Err(CacheError::UnsupportedCog {
            path: remote_path.clone(),
            reason: format!(
                "expected 512x512 tiled COG, got {}x{}",
                metadata.tile_width, metadata.tile_height
            ),
        });
    }
    let expected_tiles = metadata.tiles_across() as usize * metadata.tiles_down() as usize;
    if metadata.tile_offsets.len() != expected_tiles
        || metadata.tile_byte_counts.len() != expected_tiles
    {
        return Err(CacheError::UnsupportedCog {
            path: remote_path.clone(),
            reason: "tile offset/count arrays do not match raster dimensions".to_string(),
        });
    }
    let valid_sample = match kind {
        RasterKind::FlowDir => {
            matches!(metadata.sample_type, CogSampleType::U8 | CogSampleType::I8)
        }
        RasterKind::FlowAcc => {
            matches!(
                metadata.sample_type,
                CogSampleType::F32 | CogSampleType::I32
            )
        }
    };
    if !valid_sample {
        return Err(CacheError::UnsupportedCog {
            path: remote_path.clone(),
            reason: format!(
                "{kind:?} has unsupported samples {:?}",
                metadata.sample_type
            ),
        });
    }
    if !matches!(metadata.compression, 8 | 32946) {
        return Err(CacheError::UnsupportedCog {
            path: remote_path.clone(),
            reason: format!("expected DEFLATE compression, got {}", metadata.compression),
        });
    }
    let expected_predictor = match metadata.sample_type {
        CogSampleType::U8 | CogSampleType::I8 | CogSampleType::I32 => 2,
        CogSampleType::F32 => 3,
    };
    if metadata.predictor != expected_predictor {
        return Err(CacheError::UnsupportedCog {
            path: remote_path.clone(),
            reason: format!(
                "{kind:?} expected TIFF predictor {expected_predictor}, got {}",
                metadata.predictor
            ),
        });
    }
    Ok(())
}

fn decode_window(
    reader: RangeBackedTiffReader,
    metadata: &CogMetadata,
    window: RasterPixelWindow,
    plan: &TilePlan,
    remote_path: &ObjectPath,
) -> Result<WindowData, CacheError> {
    let mut decoder = Decoder::new(reader).map_err(|source| CacheError::Tiff {
        path: remote_path.as_ref().to_string(),
        source,
    })?;

    match metadata.sample_type {
        CogSampleType::U8 => {
            let mut out = vec![0_u8; window.width as usize * window.height as usize];
            for tile in &plan.tiles {
                let decoded =
                    decoder
                        .read_chunk(tile.index)
                        .map_err(|source| CacheError::Tiff {
                            path: remote_path.as_ref().to_string(),
                            source,
                        })?;
                let DecodingResult::U8(data) = decoded else {
                    return Err(CacheError::UnsupportedCog {
                        path: remote_path.clone(),
                        reason: "decoded flow_dir tile was not u8".to_string(),
                    });
                };
                copy_tile_u8(&data, &mut out, metadata, window, tile.index);
            }
            Ok(WindowData::U8(out))
        }
        CogSampleType::I8 => {
            let mut out = vec![0_u8; window.width as usize * window.height as usize];
            for tile in &plan.tiles {
                let decoded =
                    decoder
                        .read_chunk(tile.index)
                        .map_err(|source| CacheError::Tiff {
                            path: remote_path.as_ref().to_string(),
                            source,
                        })?;
                let DecodingResult::I8(data) = decoded else {
                    return Err(CacheError::UnsupportedCog {
                        path: remote_path.clone(),
                        reason: "decoded flow_dir tile was not i8".to_string(),
                    });
                };
                let normalized = data
                    .into_iter()
                    .map(|value| value as u8)
                    .collect::<Vec<_>>();
                copy_tile_u8(&normalized, &mut out, metadata, window, tile.index);
            }
            Ok(WindowData::U8(out))
        }
        CogSampleType::F32 => {
            let nodata = metadata.nodata.parse::<f32>().ok();
            let mut out =
                vec![nodata.unwrap_or(f32::NAN); window.width as usize * window.height as usize];
            for tile in &plan.tiles {
                let decoded =
                    decoder
                        .read_chunk(tile.index)
                        .map_err(|source| CacheError::Tiff {
                            path: remote_path.as_ref().to_string(),
                            source,
                        })?;
                let DecodingResult::F32(data) = decoded else {
                    return Err(CacheError::UnsupportedCog {
                        path: remote_path.clone(),
                        reason: "decoded flow_acc tile was not f32".to_string(),
                    });
                };
                copy_tile_f32(&data, &mut out, metadata, window, tile.index);
            }
            Ok(WindowData::F32(out))
        }
        CogSampleType::I32 => {
            let nodata = metadata.nodata.parse::<i32>().ok();
            let mut out = vec![f32::NAN; window.width as usize * window.height as usize];
            for tile in &plan.tiles {
                let decoded =
                    decoder
                        .read_chunk(tile.index)
                        .map_err(|source| CacheError::Tiff {
                            path: remote_path.as_ref().to_string(),
                            source,
                        })?;
                let DecodingResult::I32(data) = decoded else {
                    return Err(CacheError::UnsupportedCog {
                        path: remote_path.clone(),
                        reason: "decoded flow_acc tile was not i32".to_string(),
                    });
                };
                let normalized = normalize_i32_accumulation(data, nodata);
                copy_tile_f32(&normalized, &mut out, metadata, window, tile.index);
            }
            Ok(WindowData::F32(out))
        }
    }
}

fn copy_tile_u8(
    tile_data: &[u8],
    out: &mut [u8],
    metadata: &CogMetadata,
    window: RasterPixelWindow,
    tile_index: u32,
) {
    let (tile_col, tile_row) = tile_col_row(metadata, tile_index);
    let (src_width, _src_height, dst_col, dst_row, copy_width, copy_height) =
        tile_copy_span(metadata, window, tile_col, tile_row);

    for row in 0..copy_height {
        let src_start = ((dst_row + row - tile_row * metadata.tile_height) * src_width
            + (dst_col - tile_col * metadata.tile_width)) as usize;
        let dst_start =
            ((dst_row + row - window.row_off) * window.width + (dst_col - window.col_off)) as usize;
        out[dst_start..dst_start + copy_width as usize]
            .copy_from_slice(&tile_data[src_start..src_start + copy_width as usize]);
    }
}

fn copy_tile_f32(
    tile_data: &[f32],
    out: &mut [f32],
    metadata: &CogMetadata,
    window: RasterPixelWindow,
    tile_index: u32,
) {
    let (tile_col, tile_row) = tile_col_row(metadata, tile_index);
    let (src_width, _src_height, dst_col, dst_row, copy_width, copy_height) =
        tile_copy_span(metadata, window, tile_col, tile_row);

    for row in 0..copy_height {
        let src_start = ((dst_row + row - tile_row * metadata.tile_height) * src_width
            + (dst_col - tile_col * metadata.tile_width)) as usize;
        let dst_start =
            ((dst_row + row - window.row_off) * window.width + (dst_col - window.col_off)) as usize;
        out[dst_start..dst_start + copy_width as usize]
            .copy_from_slice(&tile_data[src_start..src_start + copy_width as usize]);
    }
}

fn tile_col_row(metadata: &CogMetadata, tile_index: u32) -> (u32, u32) {
    let tiles_across = metadata.tiles_across();
    (tile_index % tiles_across, tile_index / tiles_across)
}

fn tile_copy_span(
    metadata: &CogMetadata,
    window: RasterPixelWindow,
    tile_col: u32,
    tile_row: u32,
) -> (u32, u32, u32, u32, u32, u32) {
    let tile_x = tile_col * metadata.tile_width;
    let tile_y = tile_row * metadata.tile_height;
    let src_width = min(metadata.tile_width, metadata.width - tile_x);
    let src_height = min(metadata.tile_height, metadata.height - tile_y);
    let dst_col = max(window.col_off, tile_x);
    let dst_row = max(window.row_off, tile_y);
    let copy_end_col = min(window.col_off + window.width, tile_x + src_width);
    let copy_end_row = min(window.row_off + window.height, tile_y + src_height);
    (
        src_width,
        src_height,
        dst_col,
        dst_row,
        copy_end_col - dst_col,
        copy_end_row - dst_row,
    )
}

enum WindowData {
    U8(Vec<u8>),
    F32(Vec<f32>),
}

/// Decoded local GeoTIFF window data.
#[cfg(feature = "test-fixtures")]
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum LocalWindowData {
    /// Unsigned byte samples.
    U8(Vec<u8>),
    /// 32-bit floating point samples, with nodata converted to NaN.
    F32(Vec<f32>),
}

/// A decoded local GeoTIFF window plus its grid placement metadata.
#[cfg(feature = "test-fixtures")]
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LocalTiffWindow {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) geo: GeoTransform,
    pub(crate) nodata: String,
    pub(crate) data: LocalWindowData,
}

/// Decode a local GeoTIFF window using the same metadata interpretation as COG materialization.
#[cfg(feature = "test-fixtures")]
pub(crate) fn read_local_geotiff_window(
    path: &Path,
    kind: RasterKind,
    bbox: &Rect<f64>,
) -> Result<LocalTiffWindow, CacheError> {
    let file = File::open(path).map_err(|source| CacheError::Io {
        op: "open",
        path: path.to_path_buf(),
        source,
    })?;
    let mut decoder = Decoder::new(file).map_err(|source| CacheError::Tiff {
        path: path.display().to_string(),
        source,
    })?;
    let metadata = read_local_metadata(&mut decoder, path)?;
    let valid_sample = match kind {
        RasterKind::FlowDir => {
            matches!(metadata.sample_type, CogSampleType::U8 | CogSampleType::I8)
        }
        RasterKind::FlowAcc => {
            matches!(
                metadata.sample_type,
                CogSampleType::F32 | CogSampleType::I32
            )
        }
    };
    if !valid_sample {
        return Err(CacheError::UnsupportedCog {
            path: ObjectPath::from(path.display().to_string()),
            reason: format!(
                "{kind:?} has unsupported samples {:?}",
                metadata.sample_type
            ),
        });
    }

    let window = RasterPixelWindow::from_bbox(&metadata, bbox).map_err(|reason| {
        CacheError::UnsupportedCog {
            path: ObjectPath::from(path.display().to_string()),
            reason,
        }
    })?;
    let geo = GeoTransform::new(
        NativeCoord::new(
            metadata.origin_x + f64::from(window.col_off) * metadata.pixel_width,
            metadata.origin_y + f64::from(window.row_off) * metadata.pixel_height,
        ),
        metadata.pixel_width,
        metadata.pixel_height,
    );

    let data = match (
        metadata.sample_type,
        decoder.read_image().map_err(|source| CacheError::Tiff {
            path: path.display().to_string(),
            source,
        })?,
    ) {
        (CogSampleType::U8, DecodingResult::U8(values)) => {
            LocalWindowData::U8(crop_window(&values, &metadata, window))
        }
        (CogSampleType::I8, DecodingResult::I8(values)) => {
            let values = values
                .into_iter()
                .map(|value| value as u8)
                .collect::<Vec<_>>();
            LocalWindowData::U8(crop_window(&values, &metadata, window))
        }
        (CogSampleType::F32, DecodingResult::F32(values)) => {
            let nodata = metadata.nodata.parse::<f32>().ok();
            let values = replace_nodata_with_nan(values, nodata);
            LocalWindowData::F32(crop_window(&values, &metadata, window))
        }
        (CogSampleType::I32, DecodingResult::I32(values)) => {
            let nodata = metadata.nodata.parse::<i32>().ok();
            let values = normalize_i32_accumulation(values, nodata);
            LocalWindowData::F32(crop_window(&values, &metadata, window))
        }
        (CogSampleType::U8, other) => {
            return Err(CacheError::UnsupportedCog {
                path: ObjectPath::from(path.display().to_string()),
                reason: format!("decoded flow_dir image was not u8: {other:?}"),
            });
        }
        (CogSampleType::F32, other) => {
            return Err(CacheError::UnsupportedCog {
                path: ObjectPath::from(path.display().to_string()),
                reason: format!("decoded flow_acc image was not f32: {other:?}"),
            });
        }
        (CogSampleType::I8, other) => {
            return Err(CacheError::UnsupportedCog {
                path: ObjectPath::from(path.display().to_string()),
                reason: format!("decoded flow_dir image was not i8: {other:?}"),
            });
        }
        (CogSampleType::I32, other) => {
            return Err(CacheError::UnsupportedCog {
                path: ObjectPath::from(path.display().to_string()),
                reason: format!("decoded flow_acc image was not i32: {other:?}"),
            });
        }
    };

    Ok(LocalTiffWindow {
        width: window.width,
        height: window.height,
        geo,
        nodata: normalized_nodata(&metadata),
        data,
    })
}

#[cfg(feature = "test-fixtures")]
fn read_local_metadata(
    decoder: &mut Decoder<File>,
    path: &Path,
) -> Result<CogMetadata, CacheError> {
    let (width, height) = decoder.dimensions().map_err(|source| CacheError::Tiff {
        path: path.display().to_string(),
        source,
    })?;
    let color_type = decoder.colortype().map_err(|source| CacheError::Tiff {
        path: path.display().to_string(),
        source,
    })?;
    let sample_formats = decoder
        .find_tag_unsigned_vec::<u16>(Tag::SampleFormat)
        .map_err(|source| CacheError::Tiff {
            path: path.display().to_string(),
            source,
        })?
        .unwrap_or_else(|| vec![1]);
    let sample_type = match (color_type, sample_formats.as_slice()) {
        (tiff::ColorType::Gray(8), [1]) => CogSampleType::U8,
        (tiff::ColorType::Gray(8), [2]) => CogSampleType::I8,
        (tiff::ColorType::Gray(32), [3]) => CogSampleType::F32,
        (tiff::ColorType::Gray(32), [2]) => CogSampleType::I32,
        (other, formats) => {
            return Err(CacheError::UnsupportedCog {
                path: ObjectPath::from(path.display().to_string()),
                reason: format!("unsupported sample layout: {other:?} sample_format={formats:?}"),
            });
        }
    };
    let scale = decoder
        .get_tag_f64_vec(MODEL_PIXEL_SCALE_TAG)
        .map_err(|source| CacheError::Tiff {
            path: path.display().to_string(),
            source,
        })?;
    let tiepoint = decoder
        .get_tag_f64_vec(MODEL_TIEPOINT_TAG)
        .map_err(|source| CacheError::Tiff {
            path: path.display().to_string(),
            source,
        })?;
    if scale.len() < 2 || tiepoint.len() < 6 {
        return Err(CacheError::UnsupportedCog {
            path: ObjectPath::from(path.display().to_string()),
            reason: "missing GeoTIFF model scale or tiepoint values".to_string(),
        });
    }
    let nodata = decoder
        .get_tag_ascii_string(GDAL_NODATA_TAG)
        .unwrap_or_else(|_| match sample_type {
            CogSampleType::U8 => "255".to_string(),
            CogSampleType::I8 => "-1".to_string(),
            CogSampleType::F32 => "-1".to_string(),
            CogSampleType::I32 => "-1".to_string(),
        });
    let (tile_width, tile_height) = decoder.chunk_dimensions();
    let origin_x = tiepoint[3] - tiepoint[0] * scale[0];
    let origin_y = tiepoint[4] + tiepoint[1] * scale[1];

    Ok(CogMetadata {
        width,
        height,
        tile_width,
        tile_height,
        origin_x,
        origin_y,
        pixel_width: scale[0],
        pixel_height: -scale[1],
        nodata,
        sample_type,
        compression: 1,
        predictor: 1,
        tile_offsets: Vec::new(),
        tile_byte_counts: Vec::new(),
    })
}

#[cfg(feature = "test-fixtures")]
fn crop_window<T: Copy>(values: &[T], metadata: &CogMetadata, window: RasterPixelWindow) -> Vec<T> {
    let mut out = Vec::with_capacity(window.width as usize * window.height as usize);
    for row in window.row_off..window.row_off + window.height {
        let start = (row * metadata.width + window.col_off) as usize;
        out.extend_from_slice(&values[start..start + window.width as usize]);
    }
    out
}

#[cfg(feature = "test-fixtures")]
fn replace_nodata_with_nan(mut data: Vec<f32>, nodata: Option<f32>) -> Vec<f32> {
    if let Some(nodata) = nodata
        && !nodata.is_nan()
    {
        for value in &mut data {
            if *value == nodata {
                *value = f32::NAN;
            }
        }
    }
    data
}

fn normalize_i32_accumulation(data: Vec<i32>, nodata: Option<i32>) -> Vec<f32> {
    data.into_iter()
        .map(|value| {
            if Some(value) == nodata {
                f32::NAN
            } else {
                value as f32
            }
        })
        .collect()
}

fn normalized_nodata(metadata: &CogMetadata) -> String {
    match metadata.sample_type {
        CogSampleType::I8 => metadata
            .nodata
            .parse::<i8>()
            .map(|value| (value as u8).to_string())
            .unwrap_or_else(|_| "255".to_string()),
        CogSampleType::I32 => "nan".to_string(),
        CogSampleType::U8 | CogSampleType::F32 => metadata.nodata.clone(),
    }
}

fn write_window_geotiff(
    canonical: &Path,
    metadata: &CogMetadata,
    window: RasterPixelWindow,
    data: &WindowData,
    remote_path: &ObjectPath,
) -> Result<(), CacheError> {
    let parent = canonical.parent().ok_or_else(|| CacheError::Io {
        op: "parent",
        path: canonical.to_path_buf(),
        source: std::io::Error::new(ErrorKind::InvalidInput, "cache path has no parent"),
    })?;
    std::fs::create_dir_all(parent).map_err(|source| CacheError::Io {
        op: "create_dir_all",
        path: parent.to_path_buf(),
        source,
    })?;
    let mut temp = NamedTempFile::new_in(parent).map_err(|source| CacheError::Io {
        op: "create_temp",
        path: parent.to_path_buf(),
        source,
    })?;
    let temp_path = temp.path().to_path_buf();
    {
        let file = temp.as_file_mut();
        let nodata = normalized_nodata(metadata);
        match data {
            WindowData::U8(values) => write_tiff_image::<colortype::Gray8>(
                file,
                metadata,
                window,
                values,
                &nodata,
                remote_path,
            )?,
            WindowData::F32(values) => write_tiff_image::<colortype::Gray32Float>(
                file,
                metadata,
                window,
                values,
                &nodata,
                remote_path,
            )?,
        }
        file.flush().map_err(|source| CacheError::Io {
            op: "flush",
            path: temp_path.clone(),
            source,
        })?;
        file.sync_all().map_err(|source| CacheError::Io {
            op: "sync_all",
            path: temp_path,
            source,
        })?;
    }
    match temp.persist_noclobber(canonical) {
        Ok(_) => Ok(()),
        Err(error) if error.error.kind() == ErrorKind::AlreadyExists => Ok(()),
        Err(source) => Err(CacheError::Persist { source }),
    }
}

fn write_tiff_image<C>(
    file: &mut File,
    metadata: &CogMetadata,
    window: RasterPixelWindow,
    data: &[C::Inner],
    nodata: &str,
    remote_path: &ObjectPath,
) -> Result<(), CacheError>
where
    C: colortype::ColorType,
    [C::Inner]: tiff::encoder::TiffValue,
{
    let mut encoder = TiffEncoder::new(file).map_err(|source| CacheError::Tiff {
        path: remote_path.as_ref().to_string(),
        source,
    })?;
    let mut image = encoder
        .new_image::<C>(window.width, window.height)
        .map_err(|source| CacheError::Tiff {
            path: remote_path.as_ref().to_string(),
            source,
        })?;
    let origin_x = metadata.origin_x + f64::from(window.col_off) * metadata.pixel_width;
    let origin_y = metadata.origin_y + f64::from(window.row_off) * metadata.pixel_height;
    let pixel_scale = [metadata.pixel_width, -metadata.pixel_height, 0.0];
    let tiepoint = [0.0, 0.0, 0.0, origin_x, origin_y, 0.0];
    let geo_keys: [u16; 20] = [
        1, 1, 0, 4, // header: version, revision, minor, key count
        1024, 0, 1, 2, // GTModelTypeGeoKey = Geographic
        1025, 0, 1, 1, // GTRasterTypeGeoKey = PixelIsArea
        2048, 0, 1, 4326, // GeographicTypeGeoKey = EPSG:4326
        2054, 0, 1, 9102, // GeogAngularUnitsGeoKey = degree
    ];
    image
        .encoder()
        .write_tag(MODEL_PIXEL_SCALE_TAG, &pixel_scale[..])
        .map_err(|source| CacheError::Tiff {
            path: remote_path.as_ref().to_string(),
            source,
        })?;
    image
        .encoder()
        .write_tag(MODEL_TIEPOINT_TAG, &tiepoint[..])
        .map_err(|source| CacheError::Tiff {
            path: remote_path.as_ref().to_string(),
            source,
        })?;
    image
        .encoder()
        .write_tag(GEO_KEY_DIRECTORY_TAG, &geo_keys[..])
        .map_err(|source| CacheError::Tiff {
            path: remote_path.as_ref().to_string(),
            source,
        })?;
    image
        .encoder()
        .write_tag(GDAL_NODATA_TAG, nodata)
        .map_err(|source| CacheError::Tiff {
            path: remote_path.as_ref().to_string(),
            source,
        })?;
    image.write_data(data).map_err(|source| CacheError::Tiff {
        path: remote_path.as_ref().to_string(),
        source,
    })
}

/// `Read + Seek` over a sparse set of prefetched byte ranges.
#[derive(Debug, Clone)]
pub(crate) struct RangeBackedTiffReader {
    len: u64,
    pos: u64,
    ranges: Vec<(Range<u64>, Bytes)>,
}

impl RangeBackedTiffReader {
    pub(crate) fn new(len: u64, mut ranges: Vec<(Range<u64>, Bytes)>) -> Self {
        ranges.sort_by_key(|(range, _)| range.start);
        Self {
            len,
            pos: 0,
            ranges,
        }
    }

    fn current_range(&self) -> Option<(&Range<u64>, &Bytes)> {
        self.ranges
            .iter()
            .find(|(range, _)| range.start <= self.pos && self.pos < range.end)
            .map(|(range, bytes)| (range, bytes))
    }
}

impl Read for RangeBackedTiffReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() || self.pos >= self.len {
            return Ok(0);
        }
        let Some((range, bytes)) = self.current_range() else {
            return Err(std::io::Error::new(
                ErrorKind::UnexpectedEof,
                format!("missing prefetched TIFF range at byte {}", self.pos),
            ));
        };
        let src_off = (self.pos - range.start) as usize;
        let available = bytes.len().saturating_sub(src_off);
        let wanted = min(buf.len(), available);
        buf[..wanted].copy_from_slice(&bytes[src_off..src_off + wanted]);
        self.pos += wanted as u64;
        Ok(wanted)
    }
}

impl Seek for RangeBackedTiffReader {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let new_pos = match pos {
            SeekFrom::Start(offset) => offset as i128,
            SeekFrom::End(offset) => self.len as i128 + offset as i128,
            SeekFrom::Current(offset) => self.pos as i128 + offset as i128,
        };
        if new_pos < 0 {
            return Err(std::io::Error::new(
                ErrorKind::InvalidInput,
                "cannot seek before start of TIFF",
            ));
        }
        self.pos = new_pos as u64;
        Ok(self.pos)
    }
}

#[cfg(test)]
mod tests {
    use std::fmt;
    use std::fs;
    use std::future::Future;
    use std::io;
    use std::io::Read;
    use std::ops::Range;
    use std::path::PathBuf;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, OnceLock};

    use flate2::read::ZlibDecoder;
    use futures_util::stream::BoxStream;
    use geo::coord;
    use object_store::local::LocalFileSystem;
    use object_store::{
        CopyOptions, GetOptions, GetResult, ListResult, MultipartUpload, ObjectMeta,
        PutMultipartOptions, PutOptions, PutPayload, PutResult, Result as ObjectStoreResult,
    };

    use super::*;

    const PLANETARY_INDEX_END: u64 = 24_507_158;
    /// Pre-M2 extent bound, recorded as historical fact so this invariant does not
    /// depend on a production constant that M2 deletes.
    const LEGACY_EXTENT_BOUND: u64 = 262_144;
    /// Pre-M3 window-prefix bound, recorded as historical fact for the same reason.
    const LEGACY_WINDOW_BOUND: u64 = 16_777_216;
    const PLANETARY_FILE_LEN: u64 = 24_507_159;
    const PLANETARY_TILE_COUNT: u64 = 2_041_930;

    struct CogFixtures {
        temp_dir: tempfile::TempDir,
        planetary_object_path: ObjectPath,
        classic_path: PathBuf,
    }

    fn write_u16(file: &mut File, value: u16) -> io::Result<()> {
        file.write_all(&value.to_le_bytes())
    }

    fn write_u32(file: &mut File, value: u32) -> io::Result<()> {
        file.write_all(&value.to_le_bytes())
    }

    fn write_u64(file: &mut File, value: u64) -> io::Result<()> {
        file.write_all(&value.to_le_bytes())
    }

    fn write_f64(file: &mut File, value: f64) -> io::Result<()> {
        file.write_all(&value.to_le_bytes())
    }

    fn write_bigtiff_entry(
        file: &mut File,
        tag: u16,
        field_type: u16,
        count: u64,
        value: u64,
    ) -> io::Result<()> {
        write_u16(file, tag)?;
        write_u16(file, field_type)?;
        write_u64(file, count)?;
        write_u64(file, value)
    }

    fn write_classic_tiff_entry(
        file: &mut File,
        tag: u16,
        field_type: u16,
        count: u32,
        value: u32,
    ) -> io::Result<()> {
        write_u16(file, tag)?;
        write_u16(file, field_type)?;
        write_u32(file, count)?;
        write_u32(file, value)
    }

    fn write_planetary_fixture(path: &Path) -> io::Result<()> {
        let mut file = File::create(path)?;
        file.set_len(PLANETARY_FILE_LEN)?;
        file.write_all(b"II")?;
        write_u16(&mut file, 43)?;
        write_u16(&mut file, 8)?;
        write_u16(&mut file, 0)?;
        write_u64(&mut file, 200)?;

        file.seek(SeekFrom::Start(200))?;
        write_u64(&mut file, 19)?;
        write_bigtiff_entry(&mut file, 256, 4, 1, 1_070_000)?;
        write_bigtiff_entry(&mut file, 257, 4, 1, 500_000)?;
        write_bigtiff_entry(&mut file, 258, 3, 1, 8)?;
        write_bigtiff_entry(&mut file, 259, 3, 1, 8)?;
        write_bigtiff_entry(&mut file, 262, 3, 1, 1)?;
        write_bigtiff_entry(&mut file, 277, 3, 1, 1)?;
        write_bigtiff_entry(&mut file, 282, 5, 1, (1_u64 << 32) | 1)?;
        write_bigtiff_entry(&mut file, 283, 5, 1, (1_u64 << 32) | 1)?;
        write_bigtiff_entry(&mut file, 284, 3, 1, 1)?;
        write_bigtiff_entry(&mut file, 296, 3, 1, 1)?;
        write_bigtiff_entry(&mut file, 317, 3, 1, 1)?;
        write_bigtiff_entry(&mut file, 322, 4, 1, 512)?;
        write_bigtiff_entry(&mut file, 323, 4, 1, 512)?;
        write_bigtiff_entry(&mut file, 324, 16, PLANETARY_TILE_COUNT, 3_998)?;
        write_bigtiff_entry(&mut file, 325, 4, PLANETARY_TILE_COUNT, 16_339_438)?;
        write_bigtiff_entry(&mut file, 339, 3, 1, 1)?;
        write_bigtiff_entry(&mut file, 33_550, 12, 3, 596)?;
        write_bigtiff_entry(&mut file, 33_922, 12, 6, 620)?;
        write_bigtiff_entry(
            &mut file,
            42_113,
            2,
            4,
            u64::from_le_bytes(*b"255\0\0\0\0\0"),
        )?;
        write_u64(&mut file, 0)?;

        file.seek(SeekFrom::Start(596))?;
        for value in [1.0, 1.0, 0.0] {
            write_f64(&mut file, value)?;
        }
        for value in [0.0; 6] {
            write_f64(&mut file, value)?;
        }

        file.seek(SeekFrom::Start(3_998))?;
        write_u64(&mut file, PLANETARY_INDEX_END)?;
        file.seek(SeekFrom::Start(16_339_438))?;
        write_u32(&mut file, 1)?;
        file.seek(SeekFrom::Start(PLANETARY_INDEX_END))?;
        file.write_all(&[0])
    }

    fn write_classic_fixture(path: &Path) -> io::Result<()> {
        let mut file = File::create(path)?;
        file.set_len(279)?;
        file.write_all(b"II")?;
        write_u16(&mut file, 42)?;
        write_u32(&mut file, 8)?;
        write_u16(&mut file, 16)?;
        write_classic_tiff_entry(&mut file, 256, 4, 1, 512)?;
        write_classic_tiff_entry(&mut file, 257, 4, 1, 512)?;
        write_classic_tiff_entry(&mut file, 258, 3, 1, 8)?;
        write_classic_tiff_entry(&mut file, 259, 3, 1, 8)?;
        write_classic_tiff_entry(&mut file, 262, 3, 1, 1)?;
        write_classic_tiff_entry(&mut file, 277, 3, 1, 1)?;
        write_classic_tiff_entry(&mut file, 284, 3, 1, 1)?;
        write_classic_tiff_entry(&mut file, 317, 3, 1, 1)?;
        write_classic_tiff_entry(&mut file, 322, 4, 1, 512)?;
        write_classic_tiff_entry(&mut file, 323, 4, 1, 512)?;
        write_classic_tiff_entry(&mut file, 324, 4, 1, 278)?;
        write_classic_tiff_entry(&mut file, 325, 4, 1, 1)?;
        write_classic_tiff_entry(&mut file, 339, 3, 1, 1)?;
        write_classic_tiff_entry(&mut file, 33_550, 12, 3, 206)?;
        write_classic_tiff_entry(&mut file, 33_922, 12, 6, 230)?;
        write_classic_tiff_entry(&mut file, 42_113, 2, 4, u32::from_le_bytes(*b"255\0"))?;
        write_u32(&mut file, 0)?;

        for value in [1.0, 1.0, 0.0] {
            write_f64(&mut file, value)?;
        }
        for value in [0.0; 6] {
            write_f64(&mut file, value)?;
        }
        file.write_all(&[0])
    }

    fn fixtures() -> &'static CogFixtures {
        static FIXTURES: OnceLock<CogFixtures> = OnceLock::new();
        FIXTURES.get_or_init(|| {
            let temp_dir =
                tempfile::TempDir::new().expect("fixture temp directory should be created");
            let planetary_object_path = ObjectPath::from("planetary.tif");
            let planetary_path = temp_dir.path().join(planetary_object_path.as_ref());
            let classic_path = temp_dir.path().join("classic.tif");
            write_planetary_fixture(&planetary_path)
                .expect("planetary BigTIFF fixture should be written");
            write_classic_fixture(&classic_path).expect("classic TIFF fixture should be written");
            CogFixtures {
                temp_dir,
                planetary_object_path,
                classic_path,
            }
        })
    }

    #[derive(Debug, Default)]
    struct CogFixtureStoreCounters {
        head_calls: AtomicUsize,
        get_range_calls: AtomicUsize,
        get_ranges_calls: AtomicUsize,
    }

    #[derive(Debug)]
    struct CogFixtureCountingStore {
        inner: Arc<dyn ObjectStore>,
        counters: Arc<CogFixtureStoreCounters>,
    }

    impl CogFixtureCountingStore {
        fn new(inner: Arc<dyn ObjectStore>) -> Self {
            Self {
                inner,
                counters: Arc::new(CogFixtureStoreCounters::default()),
            }
        }

        fn head_calls(&self) -> usize {
            self.counters.head_calls.load(Ordering::SeqCst)
        }

        fn get_range_calls(&self) -> usize {
            self.counters.get_range_calls.load(Ordering::SeqCst)
        }

        fn get_ranges_calls(&self) -> usize {
            self.counters.get_ranges_calls.load(Ordering::SeqCst)
        }
    }

    impl fmt::Display for CogFixtureCountingStore {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "CogFixtureCountingStore({})", self.inner)
        }
    }

    impl ObjectStore for CogFixtureCountingStore {
        fn put_opts<'life0, 'life1, 'async_trait>(
            &'life0 self,
            location: &'life1 ObjectPath,
            payload: PutPayload,
            opts: PutOptions,
        ) -> Pin<Box<dyn Future<Output = ObjectStoreResult<PutResult>> + Send + 'async_trait>>
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move { self.inner.put_opts(location, payload, opts).await })
        }

        fn put_multipart_opts<'life0, 'life1, 'async_trait>(
            &'life0 self,
            location: &'life1 ObjectPath,
            opts: PutMultipartOptions,
        ) -> Pin<
            Box<
                dyn Future<Output = ObjectStoreResult<Box<dyn MultipartUpload>>>
                    + Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move { self.inner.put_multipart_opts(location, opts).await })
        }

        fn get_opts<'life0, 'life1, 'async_trait>(
            &'life0 self,
            location: &'life1 ObjectPath,
            options: GetOptions,
        ) -> Pin<Box<dyn Future<Output = ObjectStoreResult<GetResult>> + Send + 'async_trait>>
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            Self: 'async_trait,
        {
            if options.head {
                self.counters.head_calls.fetch_add(1, Ordering::SeqCst);
            }
            if options.range.is_some() {
                self.counters.get_range_calls.fetch_add(1, Ordering::SeqCst);
            }
            Box::pin(async move { self.inner.get_opts(location, options).await })
        }

        fn get_ranges<'life0, 'life1, 'life2, 'async_trait>(
            &'life0 self,
            location: &'life1 ObjectPath,
            ranges: &'life2 [Range<u64>],
        ) -> Pin<Box<dyn Future<Output = ObjectStoreResult<Vec<Bytes>>> + Send + 'async_trait>>
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            'life2: 'async_trait,
            Self: 'async_trait,
        {
            self.counters
                .get_ranges_calls
                .fetch_add(1, Ordering::SeqCst);
            Box::pin(async move { self.inner.get_ranges(location, ranges).await })
        }

        fn delete_stream(
            &self,
            locations: BoxStream<'static, ObjectStoreResult<ObjectPath>>,
        ) -> BoxStream<'static, ObjectStoreResult<ObjectPath>> {
            self.inner.delete_stream(locations)
        }

        fn list(
            &self,
            prefix: Option<&ObjectPath>,
        ) -> BoxStream<'static, ObjectStoreResult<ObjectMeta>> {
            self.inner.list(prefix)
        }

        fn list_with_delimiter<'life0, 'life1, 'async_trait>(
            &'life0 self,
            prefix: Option<&'life1 ObjectPath>,
        ) -> Pin<Box<dyn Future<Output = ObjectStoreResult<ListResult>> + Send + 'async_trait>>
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move { self.inner.list_with_delimiter(prefix).await })
        }

        fn copy_opts<'life0, 'life1, 'life2, 'async_trait>(
            &'life0 self,
            from: &'life1 ObjectPath,
            to: &'life2 ObjectPath,
            options: CopyOptions,
        ) -> Pin<Box<dyn Future<Output = ObjectStoreResult<()>> + Send + 'async_trait>>
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            'life2: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move { self.inner.copy_opts(from, to, options).await })
        }
    }

    // prototype_layout : bounded TIFF bytes -> dimensions × GeoTIFF transform × lazy tile-index descriptors
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum PrototypeTiffFormat {
        Classic,
        BigTiff,
    }

    impl PrototypeTiffFormat {
        fn magic(self) -> u16 {
            match self {
                Self::Classic => 42,
                Self::BigTiff => 43,
            }
        }

        fn header_width(self) -> u64 {
            match self {
                Self::Classic => 8,
                Self::BigTiff => 16,
            }
        }

        fn count_width(self) -> usize {
            match self {
                Self::Classic => 2,
                Self::BigTiff => 8,
            }
        }

        fn entry_width(self) -> usize {
            match self {
                Self::Classic => 12,
                Self::BigTiff => 20,
            }
        }

        fn inline_value_width(self) -> usize {
            match self {
                Self::Classic => 4,
                Self::BigTiff => 8,
            }
        }

        fn offset_width(self) -> usize {
            match self {
                Self::Classic => 4,
                Self::BigTiff => 8,
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum PrototypeIndexStorage {
        InlineScalar(u64),
        OutOfLine(u64),
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct PrototypeIndexDescriptor {
        field_type: u16,
        element_width: u64,
        count: u64,
        storage: PrototypeIndexStorage,
    }

    impl PrototypeIndexDescriptor {
        fn byte_extent(self) -> io::Result<Option<Range<u64>>> {
            let PrototypeIndexStorage::OutOfLine(offset) = self.storage else {
                return Ok(None);
            };
            let byte_count = self
                .count
                .checked_mul(self.element_width)
                .ok_or_else(|| io::Error::new(ErrorKind::InvalidData, "field size overflow"))?;
            let end = offset
                .checked_add(byte_count)
                .ok_or_else(|| io::Error::new(ErrorKind::InvalidData, "field end overflow"))?;
            Ok(Some(offset..end))
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq)]
    struct PrototypeLayout {
        format: PrototypeTiffFormat,
        width: u64,
        height: u64,
        scale: [f64; 3],
        tiepoint: [f64; 6],
        tile_offsets: PrototypeIndexDescriptor,
        tile_byte_counts: PrototypeIndexDescriptor,
        bytes_read: usize,
    }

    #[derive(Debug, Clone, Copy)]
    struct PrototypeIfdEntry {
        tag: u16,
        field_type: u16,
        count: u64,
        value: u64,
    }

    fn prototype_field_width(field_type: u16) -> io::Result<u64> {
        match field_type {
            1 | 2 => Ok(1),
            3 => Ok(2),
            4 => Ok(4),
            5 | 12 | 16 => Ok(8),
            _ => Err(io::Error::new(
                ErrorKind::InvalidData,
                format!("unsupported prototype TIFF field type {field_type}"),
            )),
        }
    }

    fn prototype_u16(bytes: &[u8]) -> io::Result<u16> {
        bytes
            .try_into()
            .map(u16::from_le_bytes)
            .map_err(|_| io::Error::new(ErrorKind::UnexpectedEof, "missing TIFF u16"))
    }

    fn prototype_u32(bytes: &[u8]) -> io::Result<u32> {
        bytes
            .try_into()
            .map(u32::from_le_bytes)
            .map_err(|_| io::Error::new(ErrorKind::UnexpectedEof, "missing TIFF u32"))
    }

    fn prototype_u64(bytes: &[u8]) -> io::Result<u64> {
        bytes
            .try_into()
            .map(u64::from_le_bytes)
            .map_err(|_| io::Error::new(ErrorKind::UnexpectedEof, "missing TIFF u64"))
    }

    fn prototype_ifd_entry(
        format: PrototypeTiffFormat,
        bytes: &[u8],
    ) -> io::Result<PrototypeIfdEntry> {
        let tag = prototype_u16(&bytes[0..2])?;
        let field_type = prototype_u16(&bytes[2..4])?;
        let (count, value) = match format {
            PrototypeTiffFormat::Classic => (
                u64::from(prototype_u32(&bytes[4..8])?),
                u64::from(prototype_u32(&bytes[8..12])?),
            ),
            PrototypeTiffFormat::BigTiff => (
                prototype_u64(&bytes[4..12])?,
                prototype_u64(&bytes[12..20])?,
            ),
        };
        prototype_field_width(field_type)?;
        Ok(PrototypeIfdEntry {
            tag,
            field_type,
            count,
            value,
        })
    }

    fn prototype_descriptor(
        format: PrototypeTiffFormat,
        entry: PrototypeIfdEntry,
    ) -> io::Result<PrototypeIndexDescriptor> {
        let element_width = prototype_field_width(entry.field_type)?;
        let byte_count = entry
            .count
            .checked_mul(element_width)
            .ok_or_else(|| io::Error::new(ErrorKind::InvalidData, "field size overflow"))?;
        let storage = if byte_count <= format.inline_value_width() as u64 {
            PrototypeIndexStorage::InlineScalar(entry.value)
        } else {
            PrototypeIndexStorage::OutOfLine(entry.value)
        };
        Ok(PrototypeIndexDescriptor {
            field_type: entry.field_type,
            element_width,
            count: entry.count,
            storage,
        })
    }

    fn prototype_entry(entries: &[PrototypeIfdEntry], tag: u16) -> io::Result<&PrototypeIfdEntry> {
        entries
            .iter()
            .find(|entry| entry.tag == tag)
            .ok_or_else(|| {
                io::Error::new(
                    ErrorKind::InvalidData,
                    format!("missing prototype TIFF tag {tag}"),
                )
            })
    }

    fn prototype_out_of_line_range(entry: PrototypeIfdEntry) -> io::Result<Range<u64>> {
        let byte_count = entry
            .count
            .checked_mul(prototype_field_width(entry.field_type)?)
            .ok_or_else(|| io::Error::new(ErrorKind::InvalidData, "field size overflow"))?;
        let end = entry
            .value
            .checked_add(byte_count)
            .ok_or_else(|| io::Error::new(ErrorKind::InvalidData, "field end overflow"))?;
        Ok(entry.value..end)
    }

    fn prototype_doubles<const N: usize>(bytes: &[u8]) -> io::Result<[f64; N]> {
        let expected = N
            .checked_mul(8)
            .ok_or_else(|| io::Error::new(ErrorKind::InvalidData, "double size overflow"))?;
        if bytes.len() != expected {
            return Err(io::Error::new(
                ErrorKind::UnexpectedEof,
                "incomplete TIFF double field",
            ));
        }
        let mut values = [0.0; N];
        for (value, chunk) in values.iter_mut().zip(bytes.chunks_exact(8)) {
            *value =
                f64::from_le_bytes(chunk.try_into().map_err(|_| {
                    io::Error::new(ErrorKind::UnexpectedEof, "incomplete TIFF double")
                })?);
        }
        Ok(values)
    }

    async fn prototype_layout(
        store: &dyn ObjectStore,
        path: &ObjectPath,
        format: PrototypeTiffFormat,
    ) -> Result<PrototypeLayout, Box<dyn std::error::Error>> {
        let header = store.get_range(path, 0..format.header_width()).await?;
        if &header[0..2] != b"II" || prototype_u16(&header[2..4])? != format.magic() {
            return Err(io::Error::new(ErrorKind::InvalidData, "unexpected TIFF header").into());
        }
        if format == PrototypeTiffFormat::BigTiff
            && (prototype_u16(&header[4..6])? != 8 || prototype_u16(&header[6..8])? != 0)
        {
            return Err(
                io::Error::new(ErrorKind::InvalidData, "invalid BigTIFF offset header").into(),
            );
        }
        let ifd_offset = match format {
            PrototypeTiffFormat::Classic => u64::from(prototype_u32(&header[4..8])?),
            PrototypeTiffFormat::BigTiff => prototype_u64(&header[8..16])?,
        };

        let count_end = ifd_offset
            .checked_add(format.count_width() as u64)
            .ok_or_else(|| io::Error::new(ErrorKind::InvalidData, "IFD count end overflow"))?;
        let count_bytes = store.get_range(path, ifd_offset..count_end).await?;
        let entry_count = match format {
            PrototypeTiffFormat::Classic => u64::from(prototype_u16(&count_bytes)?),
            PrototypeTiffFormat::BigTiff => prototype_u64(&count_bytes)?,
        };
        let entries_len = entry_count
            .checked_mul(format.entry_width() as u64)
            .ok_or_else(|| io::Error::new(ErrorKind::InvalidData, "IFD entries size overflow"))?;
        let entries_end = count_end
            .checked_add(entries_len)
            .ok_or_else(|| io::Error::new(ErrorKind::InvalidData, "IFD entries end overflow"))?;
        let entry_bytes = store.get_range(path, count_end..entries_end).await?;
        let entries = entry_bytes
            .chunks_exact(format.entry_width())
            .map(|bytes| prototype_ifd_entry(format, bytes))
            .collect::<io::Result<Vec<_>>>()?;

        let width_entry = *prototype_entry(&entries, 256)?;
        let height_entry = *prototype_entry(&entries, 257)?;
        let scale_entry = *prototype_entry(&entries, 33_550)?;
        let tiepoint_entry = *prototype_entry(&entries, 33_922)?;
        let tile_offsets = prototype_descriptor(format, *prototype_entry(&entries, 324)?)?;
        let tile_byte_counts = prototype_descriptor(format, *prototype_entry(&entries, 325)?)?;
        let value_ranges = [
            prototype_out_of_line_range(scale_entry)?,
            prototype_out_of_line_range(tiepoint_entry)?,
        ];
        let values = store.get_ranges(path, &value_ranges).await?;

        Ok(PrototypeLayout {
            format,
            width: width_entry.value,
            height: height_entry.value,
            scale: prototype_doubles(&values[0])?,
            tiepoint: prototype_doubles(&values[1])?,
            tile_offsets,
            tile_byte_counts,
            bytes_read: header.len()
                + count_bytes.len()
                + entry_bytes.len()
                + values.iter().map(Bytes::len).sum::<usize>(),
        })
    }

    // prototype_decode : zlib DEFLATE bytes × predictor 1 -> sample bytes
    fn prototype_decode(compressed: &[u8], predictor: u16) -> io::Result<Vec<u8>> {
        let mut samples = Vec::new();
        ZlibDecoder::new(compressed).read_to_end(&mut samples)?;
        match predictor {
            1 => Ok(samples),
            _ => Err(io::Error::new(
                ErrorKind::InvalidInput,
                format!("unsupported prototype predictor {predictor}"),
            )),
        }
    }

    #[tokio::test]
    async fn owned_ifd_walker_reads_classic_metadata_without_materializing_indexes() {
        let fixtures = fixtures();
        let local_store = LocalFileSystem::new_with_prefix(fixtures.temp_dir.path())
            .expect("fixture object store should be rooted");
        let inner: Arc<dyn ObjectStore> = Arc::new(local_store);
        let store = CogFixtureCountingStore::new(inner);
        let path = ObjectPath::from("classic.tif");

        let layout = prototype_layout(&store, &path, PrototypeTiffFormat::Classic)
            .await
            .expect("classic TIFF metadata should parse");

        assert_eq!(layout.format, PrototypeTiffFormat::Classic);
        assert_eq!(layout.format.count_width(), 2);
        assert_eq!(layout.format.entry_width(), 12);
        assert_eq!(layout.format.inline_value_width(), 4);
        assert_eq!(layout.format.offset_width(), 4);
        assert_eq!(layout.width, 512);
        assert_eq!(layout.height, 512);
        assert_eq!(layout.scale, [1.0, 1.0, 0.0]);
        assert_eq!(layout.tiepoint, [0.0; 6]);
        assert_eq!(
            layout.tile_offsets,
            PrototypeIndexDescriptor {
                field_type: 4,
                element_width: 4,
                count: 1,
                storage: PrototypeIndexStorage::InlineScalar(278),
            }
        );
        assert_eq!(
            layout.tile_byte_counts,
            PrototypeIndexDescriptor {
                field_type: 4,
                element_width: 4,
                count: 1,
                storage: PrototypeIndexStorage::InlineScalar(1),
            }
        );
        assert_eq!(layout.tile_offsets.byte_extent().unwrap(), None);
        assert_eq!(layout.tile_byte_counts.byte_extent().unwrap(), None);
        assert_eq!(layout.bytes_read, 274);
        assert_eq!(store.head_calls(), 0);
        assert_eq!(store.get_range_calls(), 3);
        assert_eq!(store.get_ranges_calls(), 1);
    }

    #[tokio::test]
    async fn owned_ifd_walker_reads_bigtiff_metadata_without_materializing_indexes() {
        let fixtures = fixtures();
        let local_store = LocalFileSystem::new_with_prefix(fixtures.temp_dir.path())
            .expect("fixture object store should be rooted");
        let inner: Arc<dyn ObjectStore> = Arc::new(local_store);
        let store = CogFixtureCountingStore::new(inner);

        let layout = prototype_layout(
            &store,
            &fixtures.planetary_object_path,
            PrototypeTiffFormat::BigTiff,
        )
        .await
        .expect("BigTIFF metadata should parse");

        assert_eq!(layout.format, PrototypeTiffFormat::BigTiff);
        assert_eq!(layout.format.count_width(), 8);
        assert_eq!(layout.format.entry_width(), 20);
        assert_eq!(layout.format.inline_value_width(), 8);
        assert_eq!(layout.format.offset_width(), 8);
        assert_eq!(layout.width, 1_070_000);
        assert_eq!(layout.height, 500_000);
        assert_eq!(layout.scale, [1.0, 1.0, 0.0]);
        assert_eq!(layout.tiepoint, [0.0; 6]);
        assert_eq!(
            layout.tile_offsets,
            PrototypeIndexDescriptor {
                field_type: 16,
                element_width: 8,
                count: PLANETARY_TILE_COUNT,
                storage: PrototypeIndexStorage::OutOfLine(3_998),
            }
        );
        assert_eq!(
            layout.tile_byte_counts,
            PrototypeIndexDescriptor {
                field_type: 4,
                element_width: 4,
                count: PLANETARY_TILE_COUNT,
                storage: PrototypeIndexStorage::OutOfLine(16_339_438),
            }
        );
        assert_eq!(
            layout.tile_offsets.byte_extent().unwrap(),
            Some(3_998..16_339_438)
        );
        assert_eq!(
            layout.tile_byte_counts.byte_extent().unwrap(),
            Some(16_339_438..PLANETARY_INDEX_END)
        );
        assert_eq!(layout.bytes_read, 476);
        assert!(layout.bytes_read < 3_998);
        assert_eq!(store.head_calls(), 0);
        assert_eq!(store.get_range_calls(), 3);
        assert_eq!(store.get_ranges_calls(), 1);
    }

    #[test]
    fn known_value_deflate_chunk_with_predictor_one_decodes_without_differencing() {
        let compressed = [
            120, 156, 99, 100, 98, 102, 97, 101, 99, 231, 0, 0, 0, 128, 0, 37,
        ];

        let decoded = prototype_decode(&compressed, 1).expect("zlib DEFLATE payload should decode");

        assert_eq!(decoded, [1, 2, 3, 4, 5, 6, 7, 8]);
    }

    fn metadata() -> CogMetadata {
        CogMetadata {
            width: 2048,
            height: 1024,
            tile_width: 512,
            tile_height: 512,
            origin_x: -180.0,
            origin_y: 90.0,
            pixel_width: 1.0 / 1200.0,
            pixel_height: -1.0 / 1200.0,
            nodata: "255".to_string(),
            sample_type: CogSampleType::U8,
            compression: 8,
            predictor: 2,
            tile_offsets: (0..8).map(|idx| 1000 + idx * 100).collect(),
            tile_byte_counts: vec![50; 8],
        }
    }

    #[test]
    fn planetary_fixture_exceeds_legacy_header_bounds() {
        let fixtures = fixtures();
        let mut file = File::open(
            fixtures
                .temp_dir
                .path()
                .join(fixtures.planetary_object_path.as_ref()),
        )
        .expect("planetary fixture should be readable");
        let mut tile_byte_counts_entry = [0_u8; 20];
        file.seek(SeekFrom::Start(488))
            .expect("TileByteCounts entry should be seekable");
        file.read_exact(&mut tile_byte_counts_entry)
            .expect("TileByteCounts entry should be readable");
        let tag = u16::from_le_bytes(tile_byte_counts_entry[0..2].try_into().unwrap());
        let field_type = u16::from_le_bytes(tile_byte_counts_entry[2..4].try_into().unwrap());
        let count = u64::from_le_bytes(tile_byte_counts_entry[4..12].try_into().unwrap());
        let value_offset = u64::from_le_bytes(tile_byte_counts_entry[12..20].try_into().unwrap());
        let index_end = value_offset
            .checked_add(
                count
                    .checked_mul(4)
                    .expect("index byte count should fit u64"),
            )
            .expect("index end should fit u64");

        // DURABLE generator invariant: the index exceeds 262,144 and 16,777,216 across M2 and M3.
        assert!(
            tag == 325
                && field_type == 4
                && index_end == PLANETARY_INDEX_END
                && index_end > LEGACY_EXTENT_BOUND
                && index_end > LEGACY_WINDOW_BOUND
        );
    }

    #[tokio::test]
    async fn planetary_extent_locks_bounded_header_failure() {
        let fixtures = fixtures();
        let local_store = LocalFileSystem::new_with_prefix(fixtures.temp_dir.path())
            .expect("fixture object store should be rooted");
        let inner: Arc<dyn ObjectStore> = Arc::new(local_store);
        let store = CogFixtureCountingStore::new(inner);

        let error = read_remote_extent(&store as &dyn ObjectStore, &fixtures.planetary_object_path)
            .await
            .expect_err("bounded extent read should retain the baseline failure");

        assert_eq!(store.head_calls(), 1);
        assert_eq!(store.get_range_calls(), 1);
        assert_eq!(store.get_ranges_calls(), 0);
        // TRANSITIONAL: M2 owns conversion to green success; this assertion may not be deleted.
        assert!(matches!(
            error,
            CacheError::UnsupportedCog { path, reason }
                if path == fixtures.planetary_object_path
                    && reason == "extent header too large for bounded 262144 byte range"
        ));
    }

    #[tokio::test]
    async fn planetary_window_locks_truncated_tile_byte_counts_failure() {
        let fixtures = fixtures();
        let local_store = LocalFileSystem::new_with_prefix(fixtures.temp_dir.path())
            .expect("fixture object store should be rooted");
        let inner: Arc<dyn ObjectStore> = Arc::new(local_store);
        let store = CogFixtureCountingStore::new(inner);
        let request = RasterWindowRequest::new(
            RasterKind::FlowDir,
            Rect::new(coord! { x: 0.0, y: -1.0 }, coord! { x: 1.0, y: 0.0 }),
        );

        // The [0, 16,777,216) prefix truncates TileByteCounts by exactly 7,729,942 bytes.
        let error = prepare_window(
            &store as &dyn ObjectStore,
            &fixtures.planetary_object_path,
            &request,
        )
        .await
        .expect_err("bounded window read should retain the baseline failure");

        assert_eq!(store.head_calls(), 1);
        assert_eq!(store.get_range_calls(), 1);
        assert_eq!(store.get_ranges_calls(), 0);
        // TRANSITIONAL: M3 owns conversion to green success; this assertion may not be deleted.
        assert!(matches!(
            error,
            CacheError::Tiff {
                source: tiff::TiffError::IoError(source),
                ..
            } if source.kind() == ErrorKind::UnexpectedEof
        ));
    }

    #[test]
    fn classic_fixture_round_trips_magic_and_four_byte_offsets() {
        let fixtures = fixtures();
        let bytes = fs::read(&fixtures.classic_path).expect("classic fixture should be readable");

        assert_eq!(&bytes[0..2], b"II");
        assert_eq!(u16::from_le_bytes(bytes[2..4].try_into().unwrap()), 42);
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 8);
        assert_eq!(u16::from_le_bytes(bytes[8..10].try_into().unwrap()), 16);
        assert_eq!(u16::from_le_bytes(bytes[130..132].try_into().unwrap()), 324);
        assert_eq!(u16::from_le_bytes(bytes[132..134].try_into().unwrap()), 4);
        assert_eq!(u32::from_le_bytes(bytes[134..138].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(bytes[138..142].try_into().unwrap()), 278);

        let mut decoder =
            Decoder::new(File::open(&fixtures.classic_path).unwrap()).expect("TIFF should open");
        assert_eq!(decoder.dimensions().unwrap(), (512, 512));
        assert_eq!(decoder.chunk_dimensions(), (512, 512));
    }

    #[test]
    fn bbox_to_pixel_window_clamps_and_pads() {
        let meta = metadata();
        let bbox = Rect::new(
            coord! { x: -180.0, y: 89.99 },
            coord! { x: -179.99, y: 90.0 },
        );

        let window = RasterPixelWindow::from_bbox(&meta, &bbox).unwrap();

        assert_eq!(window.col_off, 0);
        assert_eq!(window.row_off, 0);
        assert!(window.width > 12);
        assert!(window.height > 12);
    }

    #[test]
    fn tile_plan_returns_intersecting_ranges() {
        let meta = metadata();
        let window = RasterPixelWindow {
            col_off: 500,
            row_off: 500,
            width: 30,
            height: 30,
        };

        let plan = TilePlan::for_window(&meta, window);

        assert_eq!(
            plan.tiles.iter().map(|tile| tile.index).collect::<Vec<_>>(),
            vec![0, 1, 4, 5]
        );
        assert_eq!(plan.byte_count(), 200);
    }

    #[test]
    fn range_reader_reads_across_present_ranges_and_errors_on_gap() {
        let ranges = vec![
            (0..4, Bytes::from_static(b"abcd")),
            (10..14, Bytes::from_static(b"klmn")),
        ];
        let mut reader = RangeBackedTiffReader::new(20, ranges);
        let mut buf = [0_u8; 3];
        reader.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"abc");
        reader.seek(SeekFrom::Start(10)).unwrap();
        reader.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"klm");
        reader.seek(SeekFrom::Start(5)).unwrap();
        let err = reader.read(&mut buf).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::UnexpectedEof);
    }

    #[test]
    fn materialized_window_geotiff_preserves_pixels_and_transform_tags() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("window.tif");
        let meta = metadata();
        let window = RasterPixelWindow {
            col_off: 10,
            row_off: 20,
            width: 2,
            height: 2,
        };

        write_window_geotiff(
            &path,
            &meta,
            window,
            &WindowData::U8(vec![1, 2, 3, 4]),
            &ObjectPath::from("remote/flow_dir.tif"),
        )
        .unwrap();

        let mut decoder = Decoder::new(File::open(path).unwrap()).unwrap();
        assert_eq!(decoder.dimensions().unwrap(), (2, 2));
        let scale = decoder.get_tag_f64_vec(MODEL_PIXEL_SCALE_TAG).unwrap();
        let tiepoint = decoder.get_tag_f64_vec(MODEL_TIEPOINT_TAG).unwrap();
        assert_eq!(scale, vec![meta.pixel_width, -meta.pixel_height, 0.0]);
        assert_eq!(
            tiepoint,
            vec![
                0.0,
                0.0,
                0.0,
                meta.origin_x + f64::from(window.col_off) * meta.pixel_width,
                meta.origin_y + f64::from(window.row_off) * meta.pixel_height,
                0.0
            ]
        );
        match decoder.read_image().unwrap() {
            DecodingResult::U8(values) => assert_eq!(values, vec![1, 2, 3, 4]),
            other => panic!("unexpected decoding result: {other:?}"),
        }
    }
}
