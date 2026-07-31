//! Windowed COG reads for remote raster refinement.
//! remote_layout : RemoteTiff × ObjectSize → Dimensions × GeoTiffTransform × LazyTileIndexDescriptors

use std::cmp::{max, min};
use std::fs::File;
use std::io::{ErrorKind, Read, Seek, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};

use flate2::read::ZlibDecoder;
use geo::Rect;
use object_store::path::Path as ObjectPath;
use object_store::{ObjectStore, ObjectStoreExt};
use tempfile::NamedTempFile;
use tiff::decoder::Decoder;
#[cfg(feature = "test-fixtures")]
use tiff::decoder::DecodingResult;
use tiff::encoder::{TiffEncoder, colortype};
use tiff::tags::Tag;
use tracing::debug;

#[cfg(feature = "test-fixtures")]
use crate::algo::geo_transform::GeoTransform;
#[cfg(feature = "test-fixtures")]
use crate::algo::projection::NativeCoord;
use crate::error::CacheError;
use crate::session::RasterKind;

const MAX_REMOTE_IFD_ENTRIES: u64 = 4_096;
const MAX_REMOTE_IFD_ENTRY_BYTES: u64 = 65_536;
const MAX_REMOTE_METADATA_VALUE_BYTES: u64 = 65_536;
const MAX_REMOTE_ASCII_BYTES: u64 = 256;
const MAX_PLANNED_TILE_COUNT: u64 = 65_536;
const MAX_COMPRESSED_CHUNK_BYTES: u64 = 16_777_216;
const MAX_COVERED_CHUNK_BYTES: u64 = 1_073_741_824;
const MAX_DECODED_CHUNK_BYTES: u64 = 8_388_608;
const MAX_WINDOW_ALLOCATION_BYTES: u64 = 1_073_741_824;
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
    index: CogIndex,
}

#[derive(Debug, Clone, PartialEq)]
enum CogIndex {
    Remote {
        tile_offsets: IndexDescriptor,
        tile_byte_counts: IndexDescriptor,
    },
    #[cfg(feature = "test-fixtures")]
    Local,
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
pub(crate) struct TilePlan {
    indices: Vec<u32>,
}

impl TilePlan {
    pub(crate) fn for_window(
        metadata: &CogMetadata,
        window: RasterPixelWindow,
        remote_path: &ObjectPath,
    ) -> Result<Self, CacheError> {
        let first_tile_col = window.col_off / metadata.tile_width;
        let last_col = window
            .col_off
            .checked_add(window.width)
            .and_then(|value| value.checked_sub(1))
            .ok_or_else(|| remote_layout_error(remote_path, "TIFF window column overflow"))?;
        let last_tile_col = last_col / metadata.tile_width;
        let first_tile_row = window.row_off / metadata.tile_height;
        let last_row = window
            .row_off
            .checked_add(window.height)
            .and_then(|value| value.checked_sub(1))
            .ok_or_else(|| remote_layout_error(remote_path, "TIFF window row overflow"))?;
        let last_tile_row = last_row / metadata.tile_height;
        let tiles_across = metadata.tiles_across();
        let tile_cols = u64::from(last_tile_col - first_tile_col + 1);
        let tile_rows = u64::from(last_tile_row - first_tile_row + 1);
        let count = tile_cols
            .checked_mul(tile_rows)
            .ok_or_else(|| remote_layout_error(remote_path, "TIFF planned tile count overflow"))?;
        if count > MAX_PLANNED_TILE_COUNT {
            return Err(remote_layout_error(
                remote_path,
                format!(
                    "TIFF planned tile count {count} exceeds window ceiling {MAX_PLANNED_TILE_COUNT}"
                ),
            ));
        }
        let capacity = usize::try_from(count).map_err(|_| {
            remote_layout_error(remote_path, "TIFF planned tile count does not fit usize")
        })?;
        let mut indices = Vec::with_capacity(capacity);
        for tile_row in first_tile_row..=last_tile_row {
            for tile_col in first_tile_col..=last_tile_col {
                let index = tile_row
                    .checked_mul(tiles_across)
                    .and_then(|value| value.checked_add(tile_col))
                    .ok_or_else(|| remote_layout_error(remote_path, "TIFF tile index overflow"))?;
                indices.push(index);
            }
        }
        Ok(Self { indices })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedTile {
    index: u32,
    range: Range<u64>,
}

#[derive(Debug, Clone)]
struct ResolvedTilePlan {
    object_size: u64,
    tiles: Vec<ResolvedTile>,
    compressed_bytes: u64,
}

/// Header-derived plan for a remote COG window.
#[derive(Debug, Clone)]
pub(crate) struct PreparedCogWindow {
    metadata: CogMetadata,
    window: RasterPixelWindow,
    plan: ResolvedTilePlan,
    header_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TiffFormat {
    Classic,
    BigTiff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IndexStorage {
    InlineScalar(u64),
    OutOfLine(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct IndexDescriptor {
    field_type: u16,
    element_width: u64,
    count: u64,
    storage: IndexStorage,
}

impl IndexDescriptor {
    fn byte_extent(self) -> Result<Option<Range<u64>>, String> {
        let IndexStorage::OutOfLine(offset) = self.storage else {
            return Ok(None);
        };
        let byte_count = self
            .count
            .checked_mul(self.element_width)
            .ok_or_else(|| "TIFF field size overflow".to_string())?;
        let end = offset
            .checked_add(byte_count)
            .ok_or_else(|| "TIFF field end overflow".to_string())?;
        Ok(Some(offset..end))
    }
}

impl TiffFormat {
    fn inline_width(self) -> u64 {
        match self {
            Self::Classic => 4,
            Self::BigTiff => 8,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct IfdEntry {
    tag: u16,
    field_type: u16,
    count: u64,
    value: u64,
}

#[derive(Debug, Clone, PartialEq)]
struct RemoteLayout {
    format: TiffFormat,
    width: u64,
    height: u64,
    tile_width: u64,
    tile_height: u64,
    scale: [f64; 3],
    tiepoint: [f64; 6],
    nodata: String,
    sample_type: CogSampleType,
    compression: u16,
    predictor: u16,
    tile_offsets: IndexDescriptor,
    tile_byte_counts: IndexDescriptor,
    bytes_read: usize,
}

fn remote_layout_error(path: &ObjectPath, reason: impl Into<String>) -> CacheError {
    CacheError::UnsupportedCog {
        path: path.clone(),
        reason: reason.into(),
    }
}

fn checked_remote_range(
    path: &ObjectPath,
    start: u64,
    length: u64,
    object_size: u64,
) -> Result<Range<u64>, CacheError> {
    let end = start
        .checked_add(length)
        .ok_or_else(|| remote_layout_error(path, "TIFF range end overflow"))?;
    if end > object_size {
        return Err(remote_layout_error(
            path,
            format!("TIFF range {start}..{end} exceeds object size {object_size}"),
        ));
    }
    Ok(start..end)
}

fn checked_remote_length(
    path: &ObjectPath,
    length: u64,
    ceiling: u64,
    quantity: &str,
) -> Result<usize, CacheError> {
    if length > ceiling {
        return Err(remote_layout_error(
            path,
            format!("TIFF {quantity} {length} exceeds parser ceiling {ceiling}"),
        ));
    }
    usize::try_from(length).map_err(|_| {
        remote_layout_error(path, format!("TIFF {quantity} {length} does not fit usize"))
    })
}

fn checked_remote_bytes_read(
    path: &ObjectPath,
    lengths: impl IntoIterator<Item = usize>,
) -> Result<usize, CacheError> {
    lengths.into_iter().try_fold(0_usize, |total, length| {
        total
            .checked_add(length)
            .ok_or_else(|| remote_layout_error(path, "TIFF bytes-read total overflow"))
    })
}

fn remote_u16(path: &ObjectPath, bytes: &[u8]) -> Result<u16, CacheError> {
    bytes
        .try_into()
        .map(u16::from_le_bytes)
        .map_err(|_| remote_layout_error(path, "missing TIFF u16"))
}

fn remote_u32(path: &ObjectPath, bytes: &[u8]) -> Result<u32, CacheError> {
    bytes
        .try_into()
        .map(u32::from_le_bytes)
        .map_err(|_| remote_layout_error(path, "missing TIFF u32"))
}

fn remote_u64(path: &ObjectPath, bytes: &[u8]) -> Result<u64, CacheError> {
    bytes
        .try_into()
        .map(u64::from_le_bytes)
        .map_err(|_| remote_layout_error(path, "missing TIFF u64"))
}

fn remote_field_width(path: &ObjectPath, field_type: u16) -> Result<u64, CacheError> {
    match field_type {
        1 | 2 => Ok(1),
        3 => Ok(2),
        4 => Ok(4),
        5 | 12 | 16 => Ok(8),
        _ => Err(remote_layout_error(
            path,
            format!("unsupported TIFF field type {field_type}"),
        )),
    }
}

fn remote_ifd_entry(
    path: &ObjectPath,
    format: TiffFormat,
    bytes: &[u8],
) -> Result<IfdEntry, CacheError> {
    let tag = remote_u16(path, &bytes[0..2])?;
    let field_type = remote_u16(path, &bytes[2..4])?;
    let (count, value) = match format {
        TiffFormat::Classic => (
            u64::from(remote_u32(path, &bytes[4..8])?),
            u64::from(remote_u32(path, &bytes[8..12])?),
        ),
        TiffFormat::BigTiff => (
            remote_u64(path, &bytes[4..12])?,
            remote_u64(path, &bytes[12..20])?,
        ),
    };
    remote_field_width(path, field_type)?;
    Ok(IfdEntry {
        tag,
        field_type,
        count,
        value,
    })
}

fn remote_entry<'a>(
    path: &ObjectPath,
    entries: &'a [IfdEntry],
    tag: u16,
) -> Result<&'a IfdEntry, CacheError> {
    entries
        .iter()
        .find(|entry| entry.tag == tag)
        .ok_or_else(|| remote_layout_error(path, format!("missing TIFF tag {tag}")))
}

fn remote_inline_scalar(
    path: &ObjectPath,
    entries: &[IfdEntry],
    tag: u16,
) -> Result<u64, CacheError> {
    let entry = remote_entry(path, entries, tag)?;
    if !matches!(entry.field_type, 3 | 4) || entry.count != 1 {
        return Err(remote_layout_error(
            path,
            format!("TIFF tag {tag} must be one SHORT or LONG value"),
        ));
    }
    let max_value = if entry.field_type == 3 {
        u64::from(u16::MAX)
    } else {
        u64::from(u32::MAX)
    };
    if entry.value > max_value {
        return Err(remote_layout_error(
            path,
            format!("TIFF tag {tag} has non-zero scalar padding"),
        ));
    }
    Ok(entry.value)
}

async fn remote_ascii(
    store: &dyn ObjectStore,
    path: &ObjectPath,
    object_size: u64,
    format: TiffFormat,
    entries: &[IfdEntry],
    tag: u16,
) -> Result<(Option<String>, usize), CacheError> {
    let Some(entry) = entries.iter().find(|entry| entry.tag == tag) else {
        return Ok((None, 0));
    };
    if entry.field_type != 2 || entry.count == 0 {
        return Err(remote_layout_error(
            path,
            format!("TIFF tag {tag} must contain ASCII"),
        ));
    }
    if entry.count > MAX_REMOTE_ASCII_BYTES {
        return Err(CacheError::RemoteTiffAsciiTooLong {
            path: path.clone(),
            tag,
            length: entry.count,
            limit: MAX_REMOTE_ASCII_BYTES,
        });
    }
    let width = usize::try_from(entry.count)
        .map_err(|_| remote_layout_error(path, "TIFF ASCII length does not fit usize"))?;
    let (raw, fetched_bytes) = if entry.count <= format.inline_width() {
        let bytes = entry.value.to_le_bytes();
        (bytes[..width].to_vec(), 0)
    } else {
        let range = checked_remote_range(path, entry.value, entry.count, object_size)?;
        let bytes =
            store
                .get_range(path, range)
                .await
                .map_err(|source| CacheError::ObjectStore {
                    path: path.clone(),
                    source,
                })?;
        if bytes.len() != width {
            return Err(remote_layout_error(
                path,
                format!(
                    "TIFF tag {tag} ASCII value returned {} bytes, expected {width}",
                    bytes.len()
                ),
            ));
        }
        let fetched_bytes = bytes.len();
        (bytes.to_vec(), fetched_bytes)
    };
    let text = std::str::from_utf8(&raw)
        .map_err(|_| remote_layout_error(path, format!("TIFF tag {tag} is not valid ASCII")))?
        .trim_end_matches('\0')
        .to_string();
    Ok((Some(text), fetched_bytes))
}

fn remote_value_range(
    path: &ObjectPath,
    entry: IfdEntry,
    expected_count: u64,
    object_size: u64,
) -> Result<Range<u64>, CacheError> {
    if entry.field_type != 12 {
        return Err(remote_layout_error(
            path,
            format!(
                "TIFF tag {} must use DOUBLE field type 12, got {}",
                entry.tag, entry.field_type
            ),
        ));
    }
    let byte_count = entry
        .count
        .checked_mul(remote_field_width(path, entry.field_type)?)
        .ok_or_else(|| remote_layout_error(path, "TIFF field size overflow"))?;
    checked_remote_length(
        path,
        byte_count,
        MAX_REMOTE_METADATA_VALUE_BYTES,
        "metadata value bytes",
    )?;
    if entry.count != expected_count {
        return Err(remote_layout_error(
            path,
            format!(
                "TIFF tag {} must contain exactly {expected_count} DOUBLE values, got {}",
                entry.tag, entry.count
            ),
        ));
    }
    checked_remote_range(path, entry.value, byte_count, object_size)
}

fn remote_descriptor(
    path: &ObjectPath,
    format: TiffFormat,
    entry: IfdEntry,
    object_size: u64,
) -> Result<IndexDescriptor, CacheError> {
    if entry.count == 0 || !matches!(entry.field_type, 4 | 16) {
        return Err(remote_layout_error(
            path,
            format!(
                "TIFF tile-index tag {} has unsupported type {} or count {}",
                entry.tag, entry.field_type, entry.count
            ),
        ));
    }
    let element_width = remote_field_width(path, entry.field_type)?;
    let byte_count = entry
        .count
        .checked_mul(element_width)
        .ok_or_else(|| remote_layout_error(path, "TIFF tile-index size overflow"))?;
    let inline_value_width = format.inline_width();
    let storage = if byte_count <= inline_value_width {
        IndexStorage::InlineScalar(entry.value)
    } else {
        IndexStorage::OutOfLine(entry.value)
    };
    let descriptor = IndexDescriptor {
        field_type: entry.field_type,
        element_width,
        count: entry.count,
        storage,
    };
    if let Some(range) = descriptor
        .byte_extent()
        .map_err(|reason| remote_layout_error(path, reason))?
        && range.end > object_size
    {
        return Err(remote_layout_error(
            path,
            format!(
                "TIFF range {}..{} exceeds object size {object_size}",
                range.start, range.end
            ),
        ));
    }
    Ok(descriptor)
}

fn remote_doubles<const N: usize>(path: &ObjectPath, bytes: &[u8]) -> Result<[f64; N], CacheError> {
    let expected = N
        .checked_mul(8)
        .ok_or_else(|| remote_layout_error(path, "TIFF double size overflow"))?;
    if bytes.len() != expected {
        return Err(remote_layout_error(path, "incomplete TIFF double field"));
    }
    let mut values = [0.0; N];
    for (value, chunk) in values.iter_mut().zip(bytes.chunks_exact(8)) {
        *value = f64::from_le_bytes(
            chunk
                .try_into()
                .map_err(|_| remote_layout_error(path, "incomplete TIFF double"))?,
        );
    }
    Ok(values)
}

async fn read_remote_layout(
    store: &dyn ObjectStore,
    path: &ObjectPath,
    object_size: u64,
) -> Result<RemoteLayout, CacheError> {
    let header_range = checked_remote_range(path, 0, 16, object_size)?;
    let header = store
        .get_range(path, header_range)
        .await
        .map_err(|source| CacheError::ObjectStore {
            path: path.clone(),
            source,
        })?;
    if header.len() != 16 {
        return Err(remote_layout_error(path, "incomplete 16-byte TIFF header"));
    }
    if &header[0..2] != b"II" {
        return Err(remote_layout_error(
            path,
            "only little-endian TIFF byte order is supported",
        ));
    }
    let format = match remote_u16(path, &header[2..4])? {
        42 => TiffFormat::Classic,
        43 => TiffFormat::BigTiff,
        magic => {
            return Err(remote_layout_error(
                path,
                format!("unsupported TIFF magic {magic}"),
            ));
        }
    };
    if format == TiffFormat::BigTiff
        && (remote_u16(path, &header[4..6])? != 8 || remote_u16(path, &header[6..8])? != 0)
    {
        return Err(remote_layout_error(path, "invalid BigTIFF offset header"));
    }
    let ifd_offset = match format {
        TiffFormat::Classic => u64::from(remote_u32(path, &header[4..8])?),
        TiffFormat::BigTiff => remote_u64(path, &header[8..16])?,
    };
    let (count_width, entry_width) = match format {
        TiffFormat::Classic => (2, 12),
        TiffFormat::BigTiff => (8, 20),
    };

    let count_range = checked_remote_range(path, ifd_offset, count_width, object_size)?;
    let count_bytes = store
        .get_range(path, count_range.clone())
        .await
        .map_err(|source| CacheError::ObjectStore {
            path: path.clone(),
            source,
        })?;
    if count_bytes.len()
        != usize::try_from(count_width)
            .map_err(|_| remote_layout_error(path, "TIFF IFD count width does not fit usize"))?
    {
        return Err(remote_layout_error(path, "incomplete TIFF IFD count"));
    }
    let entry_count = match format {
        TiffFormat::Classic => u64::from(remote_u16(path, &count_bytes)?),
        TiffFormat::BigTiff => remote_u64(path, &count_bytes)?,
    };
    if entry_count == 0 {
        return Err(remote_layout_error(path, "TIFF IFD contains no entries"));
    }
    let entry_capacity =
        checked_remote_length(path, entry_count, MAX_REMOTE_IFD_ENTRIES, "IFD entry count")?;
    let entries_len = entry_count
        .checked_mul(entry_width)
        .ok_or_else(|| remote_layout_error(path, "TIFF IFD entries size overflow"))?;
    // At 20 bytes per BigTIFF entry, the byte ceiling binds at 3,276 entries;
    // the 4,096-entry ceiling remains reachable for 12-byte classic entries.
    let entries_len_usize = checked_remote_length(
        path,
        entries_len,
        MAX_REMOTE_IFD_ENTRY_BYTES,
        "IFD entry bytes",
    )?;
    let entries_range = checked_remote_range(path, count_range.end, entries_len, object_size)?;
    let entry_bytes = store
        .get_range(path, entries_range)
        .await
        .map_err(|source| CacheError::ObjectStore {
            path: path.clone(),
            source,
        })?;
    if entry_bytes.len() != entries_len_usize {
        return Err(remote_layout_error(path, "incomplete TIFF IFD entries"));
    }
    let entry_width = usize::try_from(entry_width)
        .map_err(|_| remote_layout_error(path, "TIFF IFD entry width does not fit usize"))?;
    let mut entries = Vec::with_capacity(entry_capacity);
    for bytes in entry_bytes.chunks_exact(entry_width) {
        entries.push(remote_ifd_entry(path, format, bytes)?);
    }

    let width = remote_inline_scalar(path, &entries, 256)?;
    let height = remote_inline_scalar(path, &entries, 257)?;
    if width == 0 || height == 0 {
        return Err(remote_layout_error(
            path,
            "TIFF dimensions must be non-zero",
        ));
    }
    let scale_range =
        remote_value_range(path, *remote_entry(path, &entries, 33_550)?, 3, object_size)?;
    let tiepoint_range =
        remote_value_range(path, *remote_entry(path, &entries, 33_922)?, 6, object_size)?;
    let tile_offsets = remote_descriptor(
        path,
        format,
        *remote_entry(path, &entries, 324)?,
        object_size,
    )?;
    let tile_byte_counts = remote_descriptor(
        path,
        format,
        *remote_entry(path, &entries, 325)?,
        object_size,
    )?;
    if tile_offsets.count != tile_byte_counts.count {
        return Err(remote_layout_error(
            path,
            format!(
                "TIFF tile-index descriptor count mismatch: TileOffsets has {} entries, TileByteCounts has {}",
                tile_offsets.count, tile_byte_counts.count
            ),
        ));
    }
    let tile_width = remote_inline_scalar(path, &entries, 322)?;
    let tile_height = remote_inline_scalar(path, &entries, 323)?;
    let bits_per_sample = remote_inline_scalar(path, &entries, 258)?;
    let compression = remote_inline_scalar(path, &entries, 259)?;
    let photometric = remote_inline_scalar(path, &entries, 262)?;
    let samples_per_pixel = remote_inline_scalar(path, &entries, 277)?;
    let planar = remote_inline_scalar(path, &entries, 284)?;
    let predictor = remote_inline_scalar(path, &entries, 317)?;
    let sample_format = remote_inline_scalar(path, &entries, 339)?;
    if photometric != 1 || samples_per_pixel != 1 || planar != 1 {
        return Err(remote_layout_error(
            path,
            "unsupported TIFF grayscale/sample/planar layout",
        ));
    }
    let sample_type = match (bits_per_sample, sample_format) {
        (8, 1) => CogSampleType::U8,
        (8, 2) => CogSampleType::I8,
        (32, 3) => CogSampleType::F32,
        (32, 2) => CogSampleType::I32,
        _ => {
            return Err(remote_layout_error(
                path,
                format!(
                    "unsupported sample layout: bits={bits_per_sample} sample_format={sample_format}"
                ),
            ));
        }
    };
    let (nodata, ascii_bytes_read) =
        remote_ascii(store, path, object_size, format, &entries, 42_113).await?;
    let nodata = nodata.unwrap_or_else(|| {
        match sample_type {
            CogSampleType::U8 => "255",
            CogSampleType::I8 | CogSampleType::F32 | CogSampleType::I32 => "-1",
        }
        .to_string()
    });
    let values = store
        .get_ranges(path, &[scale_range, tiepoint_range])
        .await
        .map_err(|source| CacheError::ObjectStore {
            path: path.clone(),
            source,
        })?;
    if values.len() != 2 {
        return Err(remote_layout_error(
            path,
            "TIFF transform range response is incomplete",
        ));
    }

    let bytes_read = checked_remote_bytes_read(
        path,
        [
            header.len(),
            count_bytes.len(),
            entry_bytes.len(),
            values[0].len(),
            values[1].len(),
            ascii_bytes_read,
        ],
    )?;

    Ok(RemoteLayout {
        format,
        width,
        height,
        tile_width,
        tile_height,
        scale: remote_doubles(path, &values[0])?,
        tiepoint: remote_doubles(path, &values[1])?,
        nodata,
        sample_type,
        compression: u16::try_from(compression)
            .map_err(|_| remote_layout_error(path, "TIFF compression does not fit u16"))?,
        predictor: u16::try_from(predictor)
            .map_err(|_| remote_layout_error(path, "TIFF predictor does not fit u16"))?,
        tile_offsets,
        tile_byte_counts,
        bytes_read,
    })
}

impl PreparedCogWindow {
    pub(crate) fn cache_fragment(&self) -> String {
        self.window.cache_fragment()
    }
}

fn metadata_from_layout(
    layout: RemoteLayout,
    remote_path: &ObjectPath,
) -> Result<CogMetadata, CacheError> {
    let width = u32::try_from(layout.width)
        .map_err(|_| remote_layout_error(remote_path, "TIFF width does not fit u32"))?;
    let height = u32::try_from(layout.height)
        .map_err(|_| remote_layout_error(remote_path, "TIFF height does not fit u32"))?;
    let tile_width = u32::try_from(layout.tile_width)
        .map_err(|_| remote_layout_error(remote_path, "TIFF tile width does not fit u32"))?;
    let tile_height = u32::try_from(layout.tile_height)
        .map_err(|_| remote_layout_error(remote_path, "TIFF tile height does not fit u32"))?;
    Ok(CogMetadata {
        width,
        height,
        tile_width,
        tile_height,
        origin_x: layout.tiepoint[3] - layout.tiepoint[0] * layout.scale[0],
        origin_y: layout.tiepoint[4] + layout.tiepoint[1] * layout.scale[1],
        pixel_width: layout.scale[0],
        pixel_height: -layout.scale[1],
        nodata: layout.nodata,
        sample_type: layout.sample_type,
        compression: layout.compression,
        predictor: layout.predictor,
        index: CogIndex::Remote {
            tile_offsets: layout.tile_offsets,
            tile_byte_counts: layout.tile_byte_counts,
        },
    })
}

fn descriptor_entry_range(
    path: &ObjectPath,
    descriptor: IndexDescriptor,
    index: u32,
    object_size: u64,
) -> Result<Option<Range<u64>>, CacheError> {
    if u64::from(index) >= descriptor.count {
        return Err(remote_layout_error(
            path,
            format!("TIFF tile-index entry {index} exceeds descriptor count"),
        ));
    }
    let IndexStorage::OutOfLine(base) = descriptor.storage else {
        if descriptor.count != 1 || index != 0 {
            return Err(remote_layout_error(
                path,
                "TIFF inline tile-index descriptor is not scalar",
            ));
        }
        return Ok(None);
    };
    let offset = u64::from(index)
        .checked_mul(descriptor.element_width)
        .and_then(|value| base.checked_add(value))
        .ok_or_else(|| remote_layout_error(path, "TIFF tile-index entry offset overflow"))?;
    checked_remote_range(path, offset, descriptor.element_width, object_size).map(Some)
}

fn remote_index_descriptors(
    metadata: &CogMetadata,
    _path: &ObjectPath,
) -> Result<(IndexDescriptor, IndexDescriptor), CacheError> {
    match metadata.index {
        CogIndex::Remote {
            tile_offsets,
            tile_byte_counts,
        } => Ok((tile_offsets, tile_byte_counts)),
        #[cfg(feature = "test-fixtures")]
        CogIndex::Local => Err(remote_layout_error(
            _path,
            "local TIFF metadata cannot resolve remote tile indexes",
        )),
    }
}

fn decode_descriptor_value(
    path: &ObjectPath,
    descriptor: IndexDescriptor,
    index: u32,
    bytes: Option<&[u8]>,
    tag: u16,
) -> Result<u64, CacheError> {
    if let IndexStorage::InlineScalar(value) = descriptor.storage {
        return Ok(value);
    }
    let bytes = bytes.ok_or_else(|| {
        remote_layout_error(
            path,
            format!("TIFF tile-index tag {tag} entry {index} response is missing"),
        )
    })?;
    let expected = usize::try_from(descriptor.element_width).map_err(|_| {
        remote_layout_error(path, "TIFF tile-index element width does not fit usize")
    })?;
    if bytes.len() != expected {
        return Err(remote_layout_error(
            path,
            format!(
                "TIFF tile-index tag {tag} entry {index} returned {} bytes, expected {expected}",
                bytes.len()
            ),
        ));
    }
    match descriptor.field_type {
        4 => Ok(u64::from(remote_u32(path, bytes)?)),
        16 => remote_u64(path, bytes),
        field_type => Err(remote_layout_error(
            path,
            format!("unsupported TIFF tile-index field type {field_type}"),
        )),
    }
}

async fn resolve_tile_plan(
    store: &dyn ObjectStore,
    path: &ObjectPath,
    metadata: &CogMetadata,
    geometry: &TilePlan,
    object_size: u64,
) -> Result<(ResolvedTilePlan, u64), CacheError> {
    let (tile_offsets, tile_byte_counts) = remote_index_descriptors(metadata, path)?;
    let mut ranges = Vec::new();
    for &index in &geometry.indices {
        if let Some(range) = descriptor_entry_range(path, tile_offsets, index, object_size)? {
            ranges.push(range);
        }
        if let Some(range) = descriptor_entry_range(path, tile_byte_counts, index, object_size)? {
            ranges.push(range);
        }
    }
    let responses = if ranges.is_empty() {
        Vec::new()
    } else {
        store
            .get_ranges(path, &ranges)
            .await
            .map_err(|source| CacheError::ObjectStore {
                path: path.clone(),
                source,
            })?
    };
    if responses.len() != ranges.len() {
        return Err(remote_layout_error(
            path,
            format!(
                "TIFF tile-index response count {}, expected {}",
                responses.len(),
                ranges.len()
            ),
        ));
    }
    let mut response_index = 0_usize;
    let mut tiles = Vec::with_capacity(geometry.indices.len());
    let mut compressed_bytes = 0_u64;
    for &index in &geometry.indices {
        let offset_bytes = if matches!(tile_offsets.storage, IndexStorage::OutOfLine(_)) {
            let response = responses.get(response_index).map(AsRef::as_ref);
            response_index += 1;
            response
        } else {
            None
        };
        let count_bytes = if matches!(tile_byte_counts.storage, IndexStorage::OutOfLine(_)) {
            let response = responses.get(response_index).map(AsRef::as_ref);
            response_index += 1;
            response
        } else {
            None
        };
        let offset = decode_descriptor_value(path, tile_offsets, index, offset_bytes, 324)?;
        let byte_count = decode_descriptor_value(path, tile_byte_counts, index, count_bytes, 325)?;
        let end = offset
            .checked_add(byte_count)
            .ok_or_else(|| remote_layout_error(path, "TIFF compressed chunk end overflow"))?;
        compressed_bytes = compressed_bytes
            .checked_add(byte_count)
            .ok_or_else(|| remote_layout_error(path, "TIFF covered compressed bytes overflow"))?;
        tiles.push(ResolvedTile {
            index,
            range: offset..end,
        });
    }
    let index_bytes = responses.iter().try_fold(0_u64, |total, response| {
        let length = u64::try_from(response.len()).map_err(|_| {
            remote_layout_error(path, "TIFF index response length does not fit u64")
        })?;
        total
            .checked_add(length)
            .ok_or_else(|| remote_layout_error(path, "TIFF index response bytes overflow"))
    })?;
    Ok((
        ResolvedTilePlan {
            object_size,
            tiles,
            compressed_bytes,
        },
        index_bytes,
    ))
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
    let object_size = object_meta.size;
    let layout = read_remote_layout(store, remote_path, object_size).await?;
    let layout_bytes = u64::try_from(layout.bytes_read)
        .map_err(|_| remote_layout_error(remote_path, "TIFF metadata bytes do not fit u64"))?;
    let metadata = metadata_from_layout(layout, remote_path)?;
    validate_merit_layout(&metadata, request.kind(), remote_path)?;
    let window = RasterPixelWindow::from_bbox(&metadata, &request.bbox).map_err(|reason| {
        CacheError::UnsupportedCog {
            path: remote_path.clone(),
            reason,
        }
    })?;
    let geometry = TilePlan::for_window(&metadata, window, remote_path)?;
    let (plan, index_bytes) =
        resolve_tile_plan(store, remote_path, &metadata, &geometry, object_size).await?;
    let header_bytes = layout_bytes
        .checked_add(index_bytes)
        .ok_or_else(|| remote_layout_error(remote_path, "TIFF header bytes overflow"))?;
    Ok(PreparedCogWindow {
        metadata,
        window,
        plan,
        header_bytes,
    })
}

/// Read only the COG layout ranges needed for raster extent selection.
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
    let layout = read_remote_layout(store, remote_path, object_size).await?;
    let origin_x = layout.tiepoint[3] - layout.tiepoint[0] * layout.scale[0];
    let origin_y = layout.tiepoint[4] + layout.tiepoint[1] * layout.scale[1];
    let pixel_width = layout.scale[0];
    let pixel_height = -layout.scale[1];
    if pixel_width <= 0.0 || pixel_height >= 0.0 {
        return Err(CacheError::UnsupportedCog {
            path: remote_path.clone(),
            reason: "only north-up rasters with positive x and negative y pixels are supported"
                .to_string(),
        });
    }
    let min_x = origin_x;
    let max_x = origin_x + layout.width as f64 * pixel_width;
    let max_y = origin_y;
    let min_y = origin_y + layout.height as f64 * pixel_height;
    Ok(CogExtent {
        rect: Rect::new(
            geo::coord! { x: min_x, y: min_y },
            geo::coord! { x: max_x, y: max_y },
        ),
    })
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
    let ranges = validate_compressed_ranges(remote_path, &prepared.plan)?;
    let compressed = store
        .get_ranges(remote_path, &ranges)
        .await
        .map_err(|source| CacheError::ObjectStore {
            path: remote_path.clone(),
            source,
        })?;
    if compressed.len() != prepared.plan.tiles.len() {
        return Err(remote_layout_error(
            remote_path,
            format!(
                "TIFF compressed response count {}, expected {}",
                compressed.len(),
                prepared.plan.tiles.len()
            ),
        ));
    }
    for (tile, bytes) in prepared.plan.tiles.iter().zip(&compressed) {
        let expected = usize::try_from(tile.range.end - tile.range.start).map_err(|_| {
            remote_layout_error(
                remote_path,
                "TIFF compressed chunk length does not fit usize",
            )
        })?;
        if bytes.len() != expected {
            return Err(remote_layout_error(
                remote_path,
                format!(
                    "TIFF compressed chunk {} returned {} bytes, expected {expected}",
                    tile.index,
                    bytes.len()
                ),
            ));
        }
    }

    let window_data = decode_window(
        &compressed,
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
        header_bytes: prepared.header_bytes,
        tile_bytes: prepared.plan.compressed_bytes,
        tile_count: prepared.plan.tiles.len(),
        window_pixels: u64::from(prepared.window.width)
            .checked_mul(u64::from(prepared.window.height))
            .ok_or_else(|| remote_layout_error(remote_path, "TIFF window pixel count overflow"))?,
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

fn validate_compressed_ranges(
    path: &ObjectPath,
    plan: &ResolvedTilePlan,
) -> Result<Vec<Range<u64>>, CacheError> {
    let mut aggregate = 0_u64;
    let mut ranges = Vec::with_capacity(plan.tiles.len());
    for tile in &plan.tiles {
        if tile.range.start >= tile.range.end {
            return Err(remote_layout_error(
                path,
                format!("TIFF compressed chunk {} has an invalid range", tile.index),
            ));
        }
        let bytes = tile
            .range
            .end
            .checked_sub(tile.range.start)
            .ok_or_else(|| {
                remote_layout_error(path, "TIFF compressed chunk range length overflow")
            })?;
        if bytes > MAX_COMPRESSED_CHUNK_BYTES {
            return Err(remote_layout_error(
                path,
                format!(
                    "TIFF compressed chunk {} bytes {bytes} exceeds window ceiling {MAX_COMPRESSED_CHUNK_BYTES}",
                    tile.index
                ),
            ));
        }
        aggregate = aggregate
            .checked_add(bytes)
            .ok_or_else(|| remote_layout_error(path, "TIFF covered compressed bytes overflow"))?;
        if aggregate > MAX_COVERED_CHUNK_BYTES {
            return Err(remote_layout_error(
                path,
                format!(
                    "TIFF covered compressed bytes {aggregate} exceeds window ceiling {MAX_COVERED_CHUNK_BYTES}"
                ),
            ));
        }
        if tile.range.end > plan.object_size {
            return Err(remote_layout_error(
                path,
                format!(
                    "TIFF range {}..{} exceeds object size {}",
                    tile.range.start, tile.range.end, plan.object_size
                ),
            ));
        }
        ranges.push(tile.range.clone());
    }
    Ok(ranges)
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
    let expected_tiles = u64::from(metadata.tiles_across())
        .checked_mul(u64::from(metadata.tiles_down()))
        .ok_or_else(|| remote_layout_error(remote_path, "TIFF tile count overflow"))?;
    let (tile_offsets, tile_byte_counts) = remote_index_descriptors(metadata, remote_path)?;
    if tile_offsets.count != expected_tiles || tile_byte_counts.count != expected_tiles {
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
    let supported = match metadata.sample_type {
        CogSampleType::U8 | CogSampleType::I8 | CogSampleType::I32 => {
            matches!(metadata.predictor, 1 | 2)
        }
        CogSampleType::F32 => matches!(metadata.predictor, 1 | 3),
    };
    if !supported {
        let sample = match metadata.sample_type {
            CogSampleType::U8 => "U8 supports TIFF predictors 1 or 2",
            CogSampleType::I8 => "I8 supports TIFF predictors 1 or 2",
            CogSampleType::I32 => "I32 supports TIFF predictors 1 or 2",
            CogSampleType::F32 => "F32 supports TIFF predictors 1 or 3",
        };
        return Err(CacheError::UnsupportedCog {
            path: remote_path.clone(),
            reason: format!("{sample}, got {}", metadata.predictor),
        });
    }
    Ok(())
}

fn window_allocation_len(
    path: &ObjectPath,
    width: u32,
    height: u32,
    sample_width: u64,
) -> Result<usize, CacheError> {
    let elements = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or_else(|| remote_layout_error(path, "TIFF window element count overflow"))?;
    let bytes = elements
        .checked_mul(sample_width)
        .ok_or_else(|| remote_layout_error(path, "TIFF window allocation bytes overflow"))?;
    if bytes > MAX_WINDOW_ALLOCATION_BYTES {
        return Err(remote_layout_error(
            path,
            format!(
                "TIFF window allocation bytes {bytes} exceeds window ceiling {MAX_WINDOW_ALLOCATION_BYTES}"
            ),
        ));
    }
    usize::try_from(elements)
        .map_err(|_| remote_layout_error(path, "TIFF window element count does not fit usize"))
}

fn direction_nodata_byte(
    sample_type: CogSampleType,
    declared: &str,
    remote_path: &ObjectPath,
) -> Result<u8, CacheError> {
    match sample_type {
        CogSampleType::U8 => declared.parse::<u8>().map_err(|_| {
            remote_layout_error(
                remote_path,
                format!("declared U8 nodata {declared:?} is not representable as u8"),
            )
        }),
        CogSampleType::I8 => declared
            .parse::<i8>()
            .map(|value| value as u8)
            .map_err(|_| {
                remote_layout_error(
                    remote_path,
                    format!("declared I8 nodata {declared:?} is not representable as i8"),
                )
            }),
        CogSampleType::F32 | CogSampleType::I32 => Err(remote_layout_error(
            remote_path,
            format!("sample type {sample_type:?} does not use byte nodata"),
        )),
    }
}

#[derive(Debug, Clone, PartialEq)]
enum OwnedTileData {
    U8(Vec<u8>),
    F32(Vec<f32>),
}

fn decode_owned_chunk(
    compressed: &[u8],
    metadata: &CogMetadata,
    tile_index: u32,
    remote_path: &ObjectPath,
) -> Result<OwnedTileData, CacheError> {
    let sample_width = match metadata.sample_type {
        CogSampleType::U8 | CogSampleType::I8 => 1_u64,
        CogSampleType::F32 | CogSampleType::I32 => 4_u64,
    };
    let expected_u64 = u64::from(metadata.tile_width)
        .checked_mul(u64::from(metadata.tile_height))
        .and_then(|value| value.checked_mul(sample_width))
        .ok_or_else(|| remote_layout_error(remote_path, "TIFF decoded chunk size overflow"))?;
    if expected_u64 > MAX_DECODED_CHUNK_BYTES {
        return Err(remote_layout_error(
            remote_path,
            format!(
                "TIFF decoded chunk bytes {expected_u64} exceeds window ceiling {MAX_DECODED_CHUNK_BYTES}"
            ),
        ));
    }
    let expected = usize::try_from(expected_u64).map_err(|_| {
        remote_layout_error(remote_path, "TIFF decoded chunk size does not fit usize")
    })?;
    let read_limit = expected_u64
        .checked_add(1)
        .ok_or_else(|| remote_layout_error(remote_path, "TIFF decoded read limit overflow"))?;
    let mut inflated = Vec::with_capacity(expected);
    ZlibDecoder::new(compressed)
        .take(read_limit)
        .read_to_end(&mut inflated)
        .map_err(|source| {
            remote_layout_error(
                remote_path,
                format!("TIFF tile {tile_index} DEFLATE decode failed: {source}"),
            )
        })?;
    if inflated.len() != expected {
        return Err(remote_layout_error(
            remote_path,
            format!(
                "TIFF tile {tile_index} decoded {} bytes, expected {expected}",
                inflated.len()
            ),
        ));
    }

    let padded_width = usize::try_from(metadata.tile_width)
        .map_err(|_| remote_layout_error(remote_path, "TIFF tile width does not fit usize"))?;
    match (metadata.sample_type, metadata.predictor) {
        (_, 1) => {}
        (CogSampleType::U8 | CogSampleType::I8, 2) => {
            for row in inflated.chunks_exact_mut(padded_width) {
                for index in 1..row.len() {
                    row[index] = row[index].wrapping_add(row[index - 1]);
                }
            }
        }
        (CogSampleType::I32, 2) => {
            let row_bytes = padded_width
                .checked_mul(4)
                .ok_or_else(|| remote_layout_error(remote_path, "TIFF predictor row overflow"))?;
            for row in inflated.chunks_exact_mut(row_bytes) {
                let mut previous = 0_i32;
                for chunk in row.chunks_exact_mut(4) {
                    let difference = i32::from_le_bytes(chunk.try_into().map_err(|_| {
                        remote_layout_error(remote_path, "incomplete TIFF I32 sample")
                    })?);
                    let value = previous.wrapping_add(difference);
                    chunk.copy_from_slice(&value.to_le_bytes());
                    previous = value;
                }
            }
        }
        (CogSampleType::F32, 3) => {
            let row_bytes = padded_width
                .checked_mul(4)
                .ok_or_else(|| remote_layout_error(remote_path, "TIFF predictor row overflow"))?;
            for row in inflated.chunks_exact_mut(row_bytes) {
                for index in 1..row.len() {
                    row[index] = row[index].wrapping_add(row[index - 1]);
                }
            }
        }
        (sample_type, predictor) => {
            return Err(remote_layout_error(
                remote_path,
                format!("unsupported TIFF predictor {predictor} for {sample_type:?}"),
            ));
        }
    }

    let (tile_col, tile_row) = tile_col_row(metadata, tile_index);
    let tile_x = tile_col
        .checked_mul(metadata.tile_width)
        .ok_or_else(|| remote_layout_error(remote_path, "TIFF tile x overflow"))?;
    let tile_y = tile_row
        .checked_mul(metadata.tile_height)
        .ok_or_else(|| remote_layout_error(remote_path, "TIFF tile y overflow"))?;
    let live_width = min(metadata.tile_width, metadata.width - tile_x);
    let live_height = min(metadata.tile_height, metadata.height - tile_y);
    let live_width_usize = usize::try_from(live_width)
        .map_err(|_| remote_layout_error(remote_path, "TIFF live tile width does not fit usize"))?;
    let live_height_usize = usize::try_from(live_height).map_err(|_| {
        remote_layout_error(remote_path, "TIFF live tile height does not fit usize")
    })?;
    let live_len = live_width_usize
        .checked_mul(live_height_usize)
        .ok_or_else(|| remote_layout_error(remote_path, "TIFF live tile size overflow"))?;

    match metadata.sample_type {
        CogSampleType::U8 | CogSampleType::I8 => {
            let mut clipped = Vec::with_capacity(live_len);
            for row in inflated.chunks_exact(padded_width).take(live_height_usize) {
                clipped.extend_from_slice(&row[..live_width_usize]);
            }
            Ok(OwnedTileData::U8(clipped))
        }
        CogSampleType::F32 => {
            let row_bytes = padded_width
                .checked_mul(4)
                .ok_or_else(|| remote_layout_error(remote_path, "TIFF F32 row size overflow"))?;
            let mut clipped = Vec::with_capacity(live_len);
            for row in inflated.chunks_exact(row_bytes).take(live_height_usize) {
                match metadata.predictor {
                    3 => {
                        let plane_width = row_bytes / 4;
                        for column in 0..live_width_usize {
                            let bits = u32::from_be_bytes([
                                row[column],
                                row[plane_width + column],
                                row[2 * plane_width + column],
                                row[3 * plane_width + column],
                            ]);
                            clipped.push(f32::from_bits(bits));
                        }
                    }
                    _ => {
                        for chunk in row.chunks_exact(4).take(live_width_usize) {
                            clipped.push(f32::from_le_bytes(chunk.try_into().map_err(|_| {
                                remote_layout_error(remote_path, "incomplete TIFF F32 sample")
                            })?));
                        }
                    }
                }
            }
            Ok(OwnedTileData::F32(clipped))
        }
        CogSampleType::I32 => {
            let row_bytes = padded_width
                .checked_mul(4)
                .ok_or_else(|| remote_layout_error(remote_path, "TIFF I32 row size overflow"))?;
            let nodata = metadata.nodata.parse::<i32>().ok();
            let mut values = Vec::with_capacity(live_len);
            for row in inflated.chunks_exact(row_bytes).take(live_height_usize) {
                for chunk in row.chunks_exact(4).take(live_width_usize) {
                    values.push(i32::from_le_bytes(chunk.try_into().map_err(|_| {
                        remote_layout_error(remote_path, "incomplete TIFF I32 sample")
                    })?));
                }
            }
            Ok(OwnedTileData::F32(normalize_i32_accumulation(
                values, nodata,
            )))
        }
    }
}

fn decode_window<B: AsRef<[u8]>>(
    compressed: &[B],
    metadata: &CogMetadata,
    window: RasterPixelWindow,
    plan: &ResolvedTilePlan,
    remote_path: &ObjectPath,
) -> Result<WindowData, CacheError> {
    match metadata.sample_type {
        CogSampleType::U8 | CogSampleType::I8 => {
            let nodata =
                direction_nodata_byte(metadata.sample_type, &metadata.nodata, remote_path)?;
            let length = window_allocation_len(remote_path, window.width, window.height, 1)?;
            let mut out = vec![nodata; length];
            for (tile, bytes) in plan.tiles.iter().zip(compressed) {
                let OwnedTileData::U8(data) =
                    decode_owned_chunk(bytes.as_ref(), metadata, tile.index, remote_path)?
                else {
                    return Err(remote_layout_error(
                        remote_path,
                        "decoded tile type mismatch",
                    ));
                };
                copy_tile_u8(&data, &mut out, metadata, window, tile.index);
            }
            Ok(WindowData::U8(out))
        }
        CogSampleType::F32 | CogSampleType::I32 => {
            let length = window_allocation_len(remote_path, window.width, window.height, 4)?;
            let nodata = metadata.nodata.parse::<f32>().ok();
            let mut out = vec![nodata.unwrap_or(f32::NAN); length];
            for (tile, bytes) in plan.tiles.iter().zip(compressed) {
                let OwnedTileData::F32(data) =
                    decode_owned_chunk(bytes.as_ref(), metadata, tile.index, remote_path)?
                else {
                    return Err(remote_layout_error(
                        remote_path,
                        "decoded tile type mismatch",
                    ));
                };
                copy_tile_f32(&data, &mut out, metadata, window, tile.index);
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
        index: CogIndex::Local,
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

#[cfg(test)]
mod tests {
    use std::fmt;
    use std::fs;
    use std::future::Future;
    use std::io;
    use std::io::{Read, SeekFrom};
    use std::ops::Range;
    use std::path::PathBuf;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};

    use bytes::Bytes;
    use flate2::Compression;
    use flate2::read::ZlibDecoder;
    use flate2::write::ZlibEncoder;
    use futures_util::stream::BoxStream;
    use geo::coord;
    use object_store::local::LocalFileSystem;
    use object_store::{
        CopyOptions, GetOptions, GetRange, GetResult, ListResult, MultipartUpload, ObjectMeta,
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
    const PLANETARY_PREFIX_BYTES: usize = 838;
    const PLANETARY_TILE_COUNT: u64 = 2_041_930;
    const PLANETARY_TILE_INDICES: [u32; 4] = [2_039_838, 2_039_839, 2_041_928, 2_041_929];
    const PLANETARY_TILE_OFFSETS: [u64; 4] = [668, 1_102, 1_536, 1_970];
    const PLANETARY_TILE_BYTE_COUNTS: [u32; 4] = [434; 4];
    const FLOW_ACC_TILE_OFFSETS: [u64; 4] = [716, 3_955, 7_204, 10_440];
    const FLOW_ACC_TILE_BYTE_COUNTS: [u32; 4] = [3_239, 3_249, 3_236, 3_236];
    const FLOW_ACC_FILE_LEN: u64 = 13_676;
    const CROSS_TILE_LIVE_DIMENSIONS: [(u32, u32); 4] =
        [(512, 512), (432, 512), (512, 288), (432, 288)];
    const REGIONAL_FILE_LEN: u64 = 16_287;
    const REGIONAL_TILE_COUNT: u64 = 1_024;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct CeilingObservations {
        planned_tile_count: u64,
        largest_compressed_chunk_bytes: u64,
        covered_compressed_bytes: u64,
        decoded_chunk_bytes: u64,
        window_allocation_bytes: u64,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct RequiredCeilingMargins {
        planned_tile_count: u64,
        largest_compressed_chunk_bytes: u64,
        covered_compressed_bytes: u64,
        decoded_chunk_bytes: u64,
        window_allocation_bytes: u64,
    }

    #[derive(Debug, Clone, PartialEq)]
    struct CrossTileFixtureOracle {
        flow_dir_bbox: Rect<f64>,
        flow_dir_window: RasterPixelWindow,
        flow_dir_tile_indices: [u32; 4],
        flow_acc_bbox: Rect<f64>,
        flow_acc_window: RasterPixelWindow,
        flow_acc_tile_indices: [u32; 4],
        live_tile_dimensions: [(u32, u32); 4],
    }

    impl CrossTileFixtureOracle {
        fn new() -> Self {
            Self {
                flow_dir_bbox: Rect::new(
                    coord! { x: 1_069_057.0, y: -499_999.0 },
                    coord! { x: 1_069_999.0, y: -499_201.0 },
                ),
                flow_dir_window: RasterPixelWindow {
                    col_off: 1_069_056,
                    row_off: 499_200,
                    width: 944,
                    height: 800,
                },
                flow_dir_tile_indices: PLANETARY_TILE_INDICES,
                flow_acc_bbox: Rect::new(
                    coord! { x: 1.0, y: -799.0 },
                    coord! { x: 943.0, y: -1.0 },
                ),
                flow_acc_window: RasterPixelWindow {
                    col_off: 0,
                    row_off: 0,
                    width: 944,
                    height: 800,
                },
                flow_acc_tile_indices: [0, 1, 2, 3],
                live_tile_dimensions: CROSS_TILE_LIVE_DIMENSIONS,
            }
        }

        fn flow_dir_bbox(&self) -> Rect<f64> {
            self.flow_dir_bbox
        }

        fn flow_dir_window(&self) -> RasterPixelWindow {
            self.flow_dir_window
        }

        fn flow_dir_tile_indices(&self) -> [u32; 4] {
            self.flow_dir_tile_indices
        }

        fn flow_acc_bbox(&self) -> Rect<f64> {
            self.flow_acc_bbox
        }

        fn flow_acc_window(&self) -> RasterPixelWindow {
            self.flow_acc_window
        }

        fn flow_acc_tile_indices(&self) -> [u32; 4] {
            self.flow_acc_tile_indices
        }

        fn live_tile_dimensions(&self) -> [(u32, u32); 4] {
            self.live_tile_dimensions
        }

        fn flow_dir_ceiling_observations(&self) -> CeilingObservations {
            CeilingObservations {
                planned_tile_count: 4,
                largest_compressed_chunk_bytes: 434,
                covered_compressed_bytes: 1_736,
                decoded_chunk_bytes: 262_144,
                window_allocation_bytes: 755_200,
            }
        }

        fn flow_dir_required_ceiling_margins(&self) -> RequiredCeilingMargins {
            RequiredCeilingMargins {
                planned_tile_count: 65_532,
                largest_compressed_chunk_bytes: 16_776_782,
                covered_compressed_bytes: 1_073_740_088,
                decoded_chunk_bytes: 8_126_464,
                window_allocation_bytes: 1_072_986_624,
            }
        }

        fn flow_acc_ceiling_observations(&self) -> CeilingObservations {
            CeilingObservations {
                planned_tile_count: 4,
                largest_compressed_chunk_bytes: 3_249,
                covered_compressed_bytes: 12_960,
                decoded_chunk_bytes: 1_048_576,
                window_allocation_bytes: 3_020_800,
            }
        }

        fn flow_acc_required_ceiling_margins(&self) -> RequiredCeilingMargins {
            RequiredCeilingMargins {
                planned_tile_count: 65_532,
                largest_compressed_chunk_bytes: 16_773_967,
                covered_compressed_bytes: 1_073_728_864,
                decoded_chunk_bytes: 7_340_032,
                window_allocation_bytes: 1_070_721_024,
            }
        }

        fn one_tile_planetary_bbox(&self) -> Rect<f64> {
            Rect::new(
                coord! { x: 1_069_057.0, y: -499_202.0 },
                coord! { x: 1_069_058.0, y: -499_201.0 },
            )
        }

        fn output_coordinates(row: u32, col: u32) -> (u32, u32, u32) {
            let tile_row = row / 512;
            let tile_col = col / 512;
            let slot = 2 * tile_row + tile_col;
            (slot, row % 512, col % 512)
        }

        fn expected_u8(&self, row: u32, col: u32) -> u8 {
            let (slot, local_row, local_col) = Self::output_coordinates(row, col);
            1 + ((2 * slot + local_col + local_row / 64) % 8) as u8
        }

        fn expected_f32(&self, row: u32, col: u32) -> f32 {
            let (slot, local_row, local_col) = Self::output_coordinates(row, col);
            (1_000 * (slot + 1) + 10 * (local_row / 64) + local_col % 16) as f32
        }
    }

    fn ceiling_observations(
        prepared: &PreparedCogWindow,
        sample_width: u64,
    ) -> CeilingObservations {
        let planned_tile_count = u64::try_from(prepared.plan.tiles.len())
            .unwrap_or_else(|_| panic!("planned tile count observation does not fit u64"));
        let mut largest_compressed_chunk_bytes = None;
        let mut covered_compressed_bytes = 0_u64;
        for tile in &prepared.plan.tiles {
            let compressed_chunk_bytes = tile
                .range
                .end
                .checked_sub(tile.range.start)
                .unwrap_or_else(|| {
                    panic!(
                        "compressed chunk byte observation underflow for tile {}",
                        tile.index
                    )
                });
            largest_compressed_chunk_bytes = Some(
                largest_compressed_chunk_bytes.map_or(compressed_chunk_bytes, |largest: u64| {
                    largest.max(compressed_chunk_bytes)
                }),
            );
            covered_compressed_bytes = covered_compressed_bytes
                .checked_add(compressed_chunk_bytes)
                .unwrap_or_else(|| panic!("covered compressed byte observation overflow"));
        }
        let largest_compressed_chunk_bytes = largest_compressed_chunk_bytes
            .unwrap_or_else(|| panic!("largest compressed chunk observation requires a tile"));
        assert_eq!(
            covered_compressed_bytes, prepared.plan.compressed_bytes,
            "independently derived covered compressed byte observation must match the production plan"
        );
        let decoded_tile_pixels = u64::from(prepared.metadata.tile_width)
            .checked_mul(u64::from(prepared.metadata.tile_height))
            .unwrap_or_else(|| panic!("decoded chunk pixel observation overflow"));
        let decoded_chunk_bytes = decoded_tile_pixels
            .checked_mul(sample_width)
            .unwrap_or_else(|| panic!("decoded chunk byte observation overflow"));
        let window_pixels = u64::from(prepared.window.width)
            .checked_mul(u64::from(prepared.window.height))
            .unwrap_or_else(|| panic!("window allocation pixel observation overflow"));
        let window_allocation_bytes = window_pixels
            .checked_mul(sample_width)
            .unwrap_or_else(|| panic!("window allocation byte observation overflow"));

        CeilingObservations {
            planned_tile_count,
            largest_compressed_chunk_bytes,
            covered_compressed_bytes,
            decoded_chunk_bytes,
            window_allocation_bytes,
        }
    }

    fn assert_required_ceiling_margins(
        observations: CeilingObservations,
        required_margins: RequiredCeilingMargins,
    ) {
        fn assert_margin(ceiling_name: &str, observed: u64, ceiling: u64, required_margin: u64) {
            assert_ne!(
                required_margin, 0,
                "{ceiling_name} required margin must be nonzero"
            );
            let actual_margin = ceiling.checked_sub(observed).unwrap_or_else(|| {
                panic!("{ceiling_name} observation {observed} exceeds ceiling {ceiling}")
            });
            assert!(
                actual_margin >= required_margin,
                "{ceiling_name}: observed {observed}, ceiling {ceiling}, actual margin {actual_margin}, required margin {required_margin}"
            );
        }

        assert_margin(
            "MAX_PLANNED_TILE_COUNT",
            observations.planned_tile_count,
            MAX_PLANNED_TILE_COUNT,
            required_margins.planned_tile_count,
        );
        assert_margin(
            "MAX_COMPRESSED_CHUNK_BYTES",
            observations.largest_compressed_chunk_bytes,
            MAX_COMPRESSED_CHUNK_BYTES,
            required_margins.largest_compressed_chunk_bytes,
        );
        assert_margin(
            "MAX_COVERED_CHUNK_BYTES",
            observations.covered_compressed_bytes,
            MAX_COVERED_CHUNK_BYTES,
            required_margins.covered_compressed_bytes,
        );
        assert_margin(
            "MAX_DECODED_CHUNK_BYTES",
            observations.decoded_chunk_bytes,
            MAX_DECODED_CHUNK_BYTES,
            required_margins.decoded_chunk_bytes,
        );
        assert_margin(
            "MAX_WINDOW_ALLOCATION_BYTES",
            observations.window_allocation_bytes,
            MAX_WINDOW_ALLOCATION_BYTES,
            required_margins.window_allocation_bytes,
        );
    }

    struct CogFixtures {
        temp_dir: tempfile::TempDir,
        planetary_object_path: ObjectPath,
        regional_object_path: ObjectPath,
        flow_acc_object_path: ObjectPath,
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

    fn compress_tile(tile: &[u8]) -> io::Result<Vec<u8>> {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(tile)?;
        encoder.finish()
    }

    fn compress_cross_tile_u8_fixture(tile: &[u8]) -> io::Result<Vec<u8>> {
        // The workspace selects zlib-rs through Parquet. Pin levels that reproduce
        // the standard-zlib default fixture sizes required by these byte layouts.
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::new(7));
        encoder.write_all(tile)?;
        encoder.finish()
    }

    fn compress_cross_tile_f32_fixture(tile: &[u8]) -> io::Result<Vec<u8>> {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
        encoder.write_all(tile)?;
        encoder.finish()
    }

    fn fixture_u8_sample(slot: u32, local_row: u32, local_col: u32) -> u8 {
        1 + ((2 * slot + local_col + local_row / 64) % 8) as u8
    }

    fn fixture_f32_sample(slot: u32, local_row: u32, local_col: u32) -> f32 {
        (1_000 * (slot + 1) + 10 * (local_row / 64) + local_col % 16) as f32
    }

    fn make_u8_fixture_tile(slot: u32) -> Vec<u8> {
        let mut tile = Vec::with_capacity(512 * 512);
        for local_row in 0..512 {
            for local_col in 0..512 {
                tile.push(fixture_u8_sample(slot, local_row, local_col));
            }
        }
        tile
    }

    fn make_f32_fixture_tile(slot: u32) -> Vec<u8> {
        let mut tile = Vec::with_capacity(512 * 512 * 4);
        for local_row in 0..512 {
            for local_col in 0..512 {
                tile.extend_from_slice(
                    &fixture_f32_sample(slot, local_row, local_col).to_le_bytes(),
                );
            }
        }
        tile
    }

    // Gross-regression alarms only; these ceilings do not define or prove boundedness.
    const REMOTE_WINDOW_BYTE_BACKSTOP: u64 = 4_096;
    const REMOTE_WINDOW_API_CALL_BACKSTOP: usize = 8;

    #[derive(Debug, PartialEq)]
    struct WindowReadCost {
        header_bytes: u64,
        tile_bytes: u64,
        tile_count: usize,
        head_calls: usize,
        non_range_get_calls: usize,
        get_range_calls: usize,
        get_ranges_calls: usize,
        requested_range_bytes: u64,
        consumed_range_bytes: u64,
        charged_non_range_object_range_bytes: u64,
        total_consumed_bytes: u64,
        total_object_store_api_calls: usize,
    }

    async fn measure_window_read_cost(
        fixtures: &CogFixtures,
        path: &ObjectPath,
        bbox: Rect<f64>,
    ) -> WindowReadCost {
        let local_store = LocalFileSystem::new_with_prefix(fixtures.temp_dir.path())
            .expect("fixture object store should be rooted");
        let store = CogFixtureCountingStore::new(Arc::new(local_store));
        let cache_temp = tempfile::TempDir::new().expect("cache temp directory should be created");
        let cache = crate::raster_cache::RemoteRasterCache::new(cache_temp.path().to_path_buf());
        let request = RasterWindowRequest::new(RasterKind::FlowDir, bbox);
        let localized = cache
            .get_or_fetch_window(&store, path, &request, "test-fabric", "0.1.0")
            .await
            .expect("cache route should materialize a bounded window");
        assert!(localized.path().exists());

        let consumed_range_bytes = store.consumed_range_bytes();
        let charged_non_range_object_range_bytes = store.charged_non_range_object_range_bytes();
        WindowReadCost {
            header_bytes: localized.header_bytes(),
            tile_bytes: localized.tile_bytes(),
            tile_count: localized.tile_count(),
            head_calls: store.head_calls(),
            non_range_get_calls: store.non_range_get_calls(),
            get_range_calls: store.get_range_calls(),
            get_ranges_calls: store.get_ranges_calls(),
            requested_range_bytes: store.requested_range_bytes(),
            consumed_range_bytes,
            charged_non_range_object_range_bytes,
            total_consumed_bytes: consumed_range_bytes + charged_non_range_object_range_bytes,
            total_object_store_api_calls: store.total_object_store_api_calls(),
        }
    }

    fn write_planetary_fixture(path: &Path) -> io::Result<()> {
        let compressed_tiles = (0..4)
            .map(|slot| compress_cross_tile_u8_fixture(&make_u8_fixture_tile(slot)))
            .collect::<io::Result<Vec<_>>>()?;
        let compressed_lengths = compressed_tiles
            .iter()
            .map(|tile| u32::try_from(tile.len()).map_err(io::Error::other))
            .collect::<io::Result<Vec<_>>>()?;
        assert_eq!(compressed_lengths, PLANETARY_TILE_BYTE_COUNTS);

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

        file.seek(SeekFrom::Start(PLANETARY_TILE_OFFSETS[0]))?;
        for tile in &compressed_tiles {
            file.write_all(tile)?;
        }
        for ((index, offset), count) in PLANETARY_TILE_INDICES
            .into_iter()
            .zip(PLANETARY_TILE_OFFSETS)
            .zip(PLANETARY_TILE_BYTE_COUNTS)
        {
            file.seek(SeekFrom::Start(3_998 + 8 * u64::from(index)))?;
            write_u64(&mut file, offset)?;
            file.seek(SeekFrom::Start(16_339_438 + 4 * u64::from(index)))?;
            write_u32(&mut file, count)?;
        }
        file.seek(SeekFrom::Start(PLANETARY_INDEX_END))?;
        file.write_all(&[0])
    }

    fn write_regional_fixture(path: &Path) -> io::Result<()> {
        let mut tile_0 = vec![0_u8; 512 * 512];
        tile_0[..8].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        let mut tile_1 = vec![0_u8; 512 * 512];
        tile_1[..8].copy_from_slice(&[8, 7, 6, 5, 4, 3, 2, 1]);
        let tile_0_zlib = compress_tile(&tile_0)?;
        let tile_1_zlib = compress_tile(&tile_1)?;
        assert!(tile_0_zlib.len() + tile_1_zlib.len() <= 3_330);
        let tile_0_offset = 668_u64;
        let tile_1_offset = tile_0_offset
            .checked_add(u64::try_from(tile_0_zlib.len()).map_err(io::Error::other)?)
            .ok_or_else(|| io::Error::other("regional tile offset overflow"))?;
        let tile_0_zlib_len = u32::try_from(tile_0_zlib.len()).map_err(io::Error::other)?;
        let tile_1_zlib_len = u32::try_from(tile_1_zlib.len()).map_err(io::Error::other)?;

        let mut file = File::create(path)?;
        file.set_len(REGIONAL_FILE_LEN)?;
        file.write_all(b"II")?;
        write_u16(&mut file, 43)?;
        write_u16(&mut file, 8)?;
        write_u16(&mut file, 0)?;
        write_u64(&mut file, 200)?;

        file.seek(SeekFrom::Start(200))?;
        write_u64(&mut file, 19)?;
        write_bigtiff_entry(&mut file, 256, 4, 1, 16_384)?;
        write_bigtiff_entry(&mut file, 257, 4, 1, 16_384)?;
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
        write_bigtiff_entry(&mut file, 324, 16, REGIONAL_TILE_COUNT, 3_998)?;
        write_bigtiff_entry(&mut file, 325, 4, REGIONAL_TILE_COUNT, 12_190)?;
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

        file.seek(SeekFrom::Start(tile_0_offset))?;
        file.write_all(&tile_0_zlib)?;
        file.write_all(&tile_1_zlib)?;
        file.seek(SeekFrom::Start(3_998))?;
        write_u64(&mut file, tile_0_offset)?;
        write_u64(&mut file, tile_1_offset)?;
        file.seek(SeekFrom::Start(12_190))?;
        write_u32(&mut file, tile_0_zlib_len)?;
        write_u32(&mut file, tile_1_zlib_len)?;
        file.seek(SeekFrom::Start(16_286))?;
        file.write_all(&[0])
    }

    fn write_flow_acc_fixture(path: &Path) -> io::Result<()> {
        let compressed_tiles = (0..4)
            .map(|slot| compress_cross_tile_f32_fixture(&make_f32_fixture_tile(slot)))
            .collect::<io::Result<Vec<_>>>()?;
        let compressed_lengths = compressed_tiles
            .iter()
            .map(|tile| u32::try_from(tile.len()).map_err(io::Error::other))
            .collect::<io::Result<Vec<_>>>()?;
        assert_eq!(compressed_lengths, FLOW_ACC_TILE_BYTE_COUNTS);

        let mut file = File::create(path)?;
        file.set_len(FLOW_ACC_FILE_LEN)?;
        file.write_all(b"II")?;
        write_u16(&mut file, 43)?;
        write_u16(&mut file, 8)?;
        write_u16(&mut file, 0)?;
        write_u64(&mut file, 200)?;

        file.seek(SeekFrom::Start(200))?;
        write_u64(&mut file, 19)?;
        write_bigtiff_entry(&mut file, 256, 4, 1, 944)?;
        write_bigtiff_entry(&mut file, 257, 4, 1, 800)?;
        write_bigtiff_entry(&mut file, 258, 3, 1, 32)?;
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
        write_bigtiff_entry(&mut file, 324, 16, 4, 668)?;
        write_bigtiff_entry(&mut file, 325, 4, 4, 700)?;
        write_bigtiff_entry(&mut file, 339, 3, 1, 3)?;
        write_bigtiff_entry(&mut file, 33_550, 12, 3, 596)?;
        write_bigtiff_entry(&mut file, 33_922, 12, 6, 620)?;
        write_bigtiff_entry(
            &mut file,
            42_113,
            2,
            3,
            u64::from_le_bytes(*b"-1\0\0\0\0\0\0"),
        )?;
        write_u64(&mut file, 0)?;

        file.seek(SeekFrom::Start(596))?;
        for value in [1.0, 1.0, 0.0] {
            write_f64(&mut file, value)?;
        }
        for value in [0.0; 6] {
            write_f64(&mut file, value)?;
        }
        for offset in FLOW_ACC_TILE_OFFSETS {
            write_u64(&mut file, offset)?;
        }
        for count in FLOW_ACC_TILE_BYTE_COUNTS {
            write_u32(&mut file, count)?;
        }
        for tile in compressed_tiles {
            file.write_all(&tile)?;
        }
        Ok(())
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
            let regional_object_path = ObjectPath::from("regional.tif");
            let regional_path = temp_dir.path().join(regional_object_path.as_ref());
            let classic_path = temp_dir.path().join("classic.tif");
            let flow_acc_object_path = ObjectPath::from("flow_acc.tif");
            let flow_acc_path = temp_dir.path().join(flow_acc_object_path.as_ref());
            write_planetary_fixture(&planetary_path)
                .expect("planetary BigTIFF fixture should be written");
            write_regional_fixture(&regional_path)
                .expect("regional BigTIFF fixture should be written");
            write_classic_fixture(&classic_path).expect("classic TIFF fixture should be written");
            write_flow_acc_fixture(&flow_acc_path)
                .expect("FlowAcc BigTIFF fixture should be written");
            CogFixtures {
                temp_dir,
                planetary_object_path,
                regional_object_path,
                flow_acc_object_path,
                classic_path,
            }
        })
    }

    #[derive(Debug, Default)]
    struct CogFixtureStoreCounters {
        head_calls: AtomicUsize,
        non_range_get_calls: AtomicUsize,
        get_range_calls: AtomicUsize,
        get_ranges_calls: AtomicUsize,
        requested_range_bytes: AtomicU64,
        consumed_range_bytes: AtomicU64,
        charged_non_range_object_range_bytes: AtomicU64,
        get_range_lengths: Mutex<Vec<u64>>,
    }

    #[derive(Debug)]
    struct CogFixtureCountingStore {
        inner: Arc<dyn ObjectStore>,
        counters: Arc<CogFixtureStoreCounters>,
        short_response_length: Option<u64>,
        short_response_used: AtomicBool,
    }

    impl CogFixtureCountingStore {
        fn new(inner: Arc<dyn ObjectStore>) -> Self {
            Self {
                inner,
                counters: Arc::new(CogFixtureStoreCounters::default()),
                short_response_length: None,
                short_response_used: AtomicBool::new(false),
            }
        }

        fn with_short_response(inner: Arc<dyn ObjectStore>, requested_length: u64) -> Self {
            Self {
                inner,
                counters: Arc::new(CogFixtureStoreCounters::default()),
                short_response_length: Some(requested_length),
                short_response_used: AtomicBool::new(false),
            }
        }

        fn head_calls(&self) -> usize {
            self.counters.head_calls.load(Ordering::SeqCst)
        }

        fn non_range_get_calls(&self) -> usize {
            self.counters.non_range_get_calls.load(Ordering::SeqCst)
        }

        fn get_range_calls(&self) -> usize {
            self.counters.get_range_calls.load(Ordering::SeqCst)
        }

        fn get_ranges_calls(&self) -> usize {
            self.counters.get_ranges_calls.load(Ordering::SeqCst)
        }

        fn requested_range_bytes(&self) -> u64 {
            self.counters.requested_range_bytes.load(Ordering::SeqCst)
        }

        fn consumed_range_bytes(&self) -> u64 {
            self.counters.consumed_range_bytes.load(Ordering::SeqCst)
        }

        fn get_range_lengths(&self) -> Vec<u64> {
            self.counters.get_range_lengths.lock().unwrap().clone()
        }

        fn charged_non_range_object_range_bytes(&self) -> u64 {
            self.counters
                .charged_non_range_object_range_bytes
                .load(Ordering::SeqCst)
        }

        fn total_object_store_api_calls(&self) -> usize {
            // One get_ranges API call is one ObjectStore accounting unit; it is not
            // asserted to be one HTTP request.
            self.head_calls()
                + self.non_range_get_calls()
                + self.get_range_calls()
                + self.get_ranges_calls()
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
            let is_range = !options.head && options.range.is_some();
            let is_non_range_get = !options.head && options.range.is_none();
            if options.head {
                self.counters.head_calls.fetch_add(1, Ordering::SeqCst);
            } else if is_non_range_get {
                self.counters
                    .non_range_get_calls
                    .fetch_add(1, Ordering::SeqCst);
            } else if let Some(range) = &options.range {
                self.counters.get_range_calls.fetch_add(1, Ordering::SeqCst);
                if let GetRange::Bounded(range) = range {
                    self.counters
                        .get_range_lengths
                        .lock()
                        .unwrap()
                        .push(range.end - range.start);
                    self.counters
                        .requested_range_bytes
                        .fetch_add(range.end - range.start, Ordering::SeqCst);
                }
            }
            Box::pin(async move {
                let result = self.inner.get_opts(location, options).await?;
                if is_range {
                    self.counters
                        .consumed_range_bytes
                        .fetch_add(result.range.end - result.range.start, Ordering::SeqCst);
                }
                if is_non_range_get {
                    // Charge the declared object range at get_opts return; this is
                    // not a measurement of bytes drained from the payload stream.
                    self.counters
                        .charged_non_range_object_range_bytes
                        .fetch_add(result.range.end - result.range.start, Ordering::SeqCst);
                }
                Ok(result)
            })
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
            let requested_bytes = ranges.iter().try_fold(0_u64, |total, range| {
                total.checked_add(range.end - range.start)
            });
            Box::pin(async move {
                let requested_bytes =
                    requested_bytes.ok_or_else(|| object_store::Error::Generic {
                        store: "CogFixtureCountingStore",
                        source: Box::new(io::Error::new(
                            ErrorKind::InvalidInput,
                            "requested range byte total overflow",
                        )),
                    })?;
                self.counters
                    .requested_range_bytes
                    .fetch_add(requested_bytes, Ordering::SeqCst);
                let mut results = self.inner.get_ranges(location, ranges).await?;
                if let Some(target) = self.short_response_length
                    && !self.short_response_used.load(Ordering::SeqCst)
                    && let Some(index) = ranges
                        .iter()
                        .position(|range| range.end - range.start == target)
                    && !results[index].is_empty()
                {
                    let shortened = results[index].len() - 1;
                    results[index] = results[index].slice(..shortened);
                    self.short_response_used.store(true, Ordering::SeqCst);
                }
                let consumed_bytes = results
                    .iter()
                    .try_fold(0_u64, |total, bytes| total.checked_add(bytes.len() as u64));
                let consumed_bytes =
                    consumed_bytes.ok_or_else(|| object_store::Error::Generic {
                        store: "CogFixtureCountingStore",
                        source: Box::new(io::Error::new(
                            ErrorKind::InvalidData,
                            "consumed range byte total overflow",
                        )),
                    })?;
                self.counters
                    .consumed_range_bytes
                    .fetch_add(consumed_bytes, Ordering::SeqCst);
                Ok(results)
            })
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

    #[tokio::test]
    async fn cog_fixture_counting_store_charges_non_range_gets() {
        let fixtures = fixtures();
        let local_store = LocalFileSystem::new_with_prefix(fixtures.temp_dir.path())
            .expect("fixture object store should be rooted");
        let store = CogFixtureCountingStore::new(Arc::new(local_store));

        store
            .get(&fixtures.regional_object_path)
            .await
            .expect("regional fixture should be readable")
            .bytes()
            .await
            .expect("regional fixture bytes should be consumed");

        assert_eq!(store.non_range_get_calls(), 1);
        assert_eq!(store.charged_non_range_object_range_bytes(), 16_287);
        assert_eq!(store.get_range_calls(), 0);
        assert_eq!(store.get_ranges_calls(), 0);
        assert_eq!(store.requested_range_bytes(), 0);
        assert_eq!(store.consumed_range_bytes(), 0);
    }

    // prototype_decode : zlib DEFLATE bytes × predictor 1 -> sample bytes
    fn prototype_decode(compressed: &[u8]) -> io::Result<Vec<u8>> {
        let mut samples = Vec::new();
        ZlibDecoder::new(compressed).read_to_end(&mut samples)?;
        Ok(samples)
    }

    #[tokio::test]
    async fn owned_ifd_walker_reads_classic_metadata_without_materializing_indexes() {
        let fixtures = fixtures();
        let local_store = LocalFileSystem::new_with_prefix(fixtures.temp_dir.path())
            .expect("fixture object store should be rooted");
        let inner: Arc<dyn ObjectStore> = Arc::new(local_store);
        let store = CogFixtureCountingStore::new(inner);
        let path = ObjectPath::from("classic.tif");

        let layout = read_remote_layout(&store, &path, 279)
            .await
            .expect("classic TIFF metadata should parse");

        assert_eq!(layout.format, TiffFormat::Classic);
        assert_eq!(layout.width, 512);
        assert_eq!(layout.height, 512);
        assert_eq!(layout.scale, [1.0, 1.0, 0.0]);
        assert_eq!(layout.tiepoint, [0.0; 6]);
        assert_eq!(
            layout.tile_offsets,
            IndexDescriptor {
                field_type: 4,
                element_width: 4,
                count: 1,
                storage: IndexStorage::InlineScalar(278),
            }
        );
        assert_eq!(
            layout.tile_byte_counts,
            IndexDescriptor {
                field_type: 4,
                element_width: 4,
                count: 1,
                storage: IndexStorage::InlineScalar(1),
            }
        );
        assert_eq!(layout.tile_offsets.byte_extent().unwrap(), None);
        assert_eq!(layout.tile_byte_counts.byte_extent().unwrap(), None);
        assert_eq!(layout.bytes_read, 282);
        assert_eq!(store.requested_range_bytes(), layout.bytes_read as u64);
        assert_eq!(store.consumed_range_bytes(), layout.bytes_read as u64);
        assert_eq!(
            (
                store.head_calls(),
                store.get_range_calls(),
                store.get_ranges_calls()
            ),
            (0, 3, 1)
        );
    }

    #[tokio::test]
    async fn owned_ifd_walker_reads_bigtiff_metadata_without_materializing_indexes() {
        let fixtures = fixtures();
        let local_store = LocalFileSystem::new_with_prefix(fixtures.temp_dir.path())
            .expect("fixture object store should be rooted");
        let inner: Arc<dyn ObjectStore> = Arc::new(local_store);
        let store = CogFixtureCountingStore::new(inner);

        let layout =
            read_remote_layout(&store, &fixtures.planetary_object_path, PLANETARY_FILE_LEN)
                .await
                .expect("BigTIFF metadata should parse");

        assert_eq!(layout.format, TiffFormat::BigTiff);
        assert_eq!(layout.width, 1_070_000);
        assert_eq!(layout.height, 500_000);
        assert_eq!(layout.scale, [1.0, 1.0, 0.0]);
        assert_eq!(layout.tiepoint, [0.0; 6]);
        assert_eq!(
            layout.tile_offsets,
            IndexDescriptor {
                field_type: 16,
                element_width: 8,
                count: PLANETARY_TILE_COUNT,
                storage: IndexStorage::OutOfLine(3_998),
            }
        );
        assert_eq!(
            layout.tile_byte_counts,
            IndexDescriptor {
                field_type: 4,
                element_width: 4,
                count: PLANETARY_TILE_COUNT,
                storage: IndexStorage::OutOfLine(16_339_438),
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
        assert_eq!(store.requested_range_bytes(), 476);
        assert_eq!(store.consumed_range_bytes(), 476);
        assert_eq!(
            (
                store.head_calls(),
                store.get_range_calls(),
                store.get_ranges_calls()
            ),
            (0, 3, 1)
        );
    }

    fn bigtiff_ifd_offset(bytes: &[u8]) -> usize {
        usize::try_from(u64::from_le_bytes(
            bytes[8..16]
                .try_into()
                .expect("BigTIFF IFD offset should be present"),
        ))
        .expect("BigTIFF IFD offset should fit usize")
    }

    fn bigtiff_entry_offset(bytes: &[u8], wanted_tag: u16) -> usize {
        let ifd_offset = bigtiff_ifd_offset(bytes);
        let entry_count = usize::try_from(u64::from_le_bytes(
            bytes[ifd_offset..ifd_offset + 8]
                .try_into()
                .expect("BigTIFF entry count should be present"),
        ))
        .expect("BigTIFF entry count should fit usize");
        (0..entry_count)
            .map(|index| ifd_offset + 8 + index * 20)
            .find(|offset| {
                u16::from_le_bytes(
                    bytes[*offset..*offset + 2]
                        .try_into()
                        .expect("BigTIFF entry tag should be present"),
                ) == wanted_tag
            })
            .expect("requested BigTIFF tag should be present")
    }

    async fn mutated_planetary_layout(
        object_size: u64,
        mutate: impl FnOnce(&mut [u8]),
    ) -> (
        Result<RemoteLayout, CacheError>,
        (usize, usize, usize, u64, u64),
    ) {
        let fixtures = fixtures();
        let mut prefix = Vec::with_capacity(PLANETARY_PREFIX_BYTES);
        File::open(
            fixtures
                .temp_dir
                .path()
                .join(fixtures.planetary_object_path.as_ref()),
        )
        .expect("planetary fixture should open")
        .take(u64::try_from(PLANETARY_PREFIX_BYTES).expect("prefix length should fit u64"))
        .read_to_end(&mut prefix)
        .expect("planetary fixture prefix should be read");
        assert_eq!(prefix.len(), PLANETARY_PREFIX_BYTES);
        mutate(&mut prefix);

        let temp_dir = tempfile::TempDir::new().expect("mutated fixture directory should exist");
        let path = ObjectPath::from("mutated.tif");
        fs::write(temp_dir.path().join(path.as_ref()), prefix)
            .expect("mutated fixture prefix should be written");
        let local_store = LocalFileSystem::new_with_prefix(temp_dir.path())
            .expect("mutated fixture object store should be rooted");
        let store = CogFixtureCountingStore::new(Arc::new(local_store));
        let result = read_remote_layout(&store, &path, object_size).await;
        let counters = (
            store.head_calls(),
            store.get_range_calls(),
            store.get_ranges_calls(),
            store.requested_range_bytes(),
            store.consumed_range_bytes(),
        );
        (result, counters)
    }

    fn assert_owned_layout_rejection(
        result: Result<RemoteLayout, CacheError>,
        expected_reason: &str,
    ) {
        match result {
            Err(CacheError::UnsupportedCog { path, reason }) => {
                assert_eq!(path, ObjectPath::from("mutated.tif"));
                assert_eq!(reason, expected_reason);
            }
            other => panic!("expected UnsupportedCog rejection, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn owned_ifd_walker_rejects_big_endian_byte_order() {
        let (result, counters) = mutated_planetary_layout(838, |bytes| {
            bytes[0..2].copy_from_slice(b"MM");
        })
        .await;

        assert_owned_layout_rejection(result, "only little-endian TIFF byte order is supported");
        assert_eq!(counters, (0, 1, 0, 16, 16));
    }

    #[tokio::test]
    async fn owned_ifd_walker_rejects_bad_magic() {
        let (result, counters) = mutated_planetary_layout(838, |bytes| {
            bytes[2..4].copy_from_slice(&41_u16.to_le_bytes());
        })
        .await;

        assert_owned_layout_rejection(result, "unsupported TIFF magic 41");
        assert_eq!(counters, (0, 1, 0, 16, 16));
    }

    #[tokio::test]
    async fn owned_ifd_walker_rejects_unsupported_field_type() {
        let (result, counters) = mutated_planetary_layout(PLANETARY_FILE_LEN, |bytes| {
            let entry_offset = bigtiff_entry_offset(bytes, 256);
            bytes[entry_offset + 2..entry_offset + 4].copy_from_slice(&99_u16.to_le_bytes());
        })
        .await;

        assert_owned_layout_rejection(result, "unsupported TIFF field type 99");
        assert_eq!(counters, (0, 3, 0, 404, 404));
    }

    #[tokio::test]
    async fn owned_ifd_walker_rejects_declared_object_too_small_for_ifd_entries() {
        let (result, counters) = mutated_planetary_layout(404, |_| {}).await;

        assert_owned_layout_rejection(result, "TIFF range 208..588 exceeds object size 404");
        assert_eq!(counters, (0, 2, 0, 24, 24));
    }

    #[tokio::test]
    async fn owned_ifd_walker_rejects_empty_ifd() {
        let (result, counters) = mutated_planetary_layout(838, |bytes| {
            let ifd_offset = bigtiff_ifd_offset(bytes);
            bytes[ifd_offset..ifd_offset + 8].copy_from_slice(&0_u64.to_le_bytes());
        })
        .await;

        assert_owned_layout_rejection(result, "TIFF IFD contains no entries");
        assert_eq!(counters, (0, 2, 0, 24, 24));
    }

    #[tokio::test]
    async fn owned_ifd_walker_rejects_out_of_range_metadata_value_offset() {
        let (result, counters) = mutated_planetary_layout(PLANETARY_FILE_LEN, |bytes| {
            let entry_offset = bigtiff_entry_offset(bytes, 33_550);
            bytes[entry_offset + 12..entry_offset + 20]
                .copy_from_slice(&PLANETARY_FILE_LEN.to_le_bytes());
        })
        .await;

        assert_owned_layout_rejection(
            result,
            "TIFF range 24507159..24507183 exceeds object size 24507159",
        );
        assert_eq!(counters, (0, 3, 0, 404, 404));
    }

    #[tokio::test]
    async fn owned_ifd_walker_rejects_descriptor_count_mismatch() {
        let (result, counters) = mutated_planetary_layout(PLANETARY_FILE_LEN, |bytes| {
            let entry_offset = bigtiff_entry_offset(bytes, 325);
            let count = u64::from_le_bytes(
                bytes[entry_offset + 4..entry_offset + 12]
                    .try_into()
                    .expect("TileByteCounts count should be present"),
            );
            let mismatched_count = count
                .checked_sub(1)
                .expect("fixture count should be positive");
            bytes[entry_offset + 4..entry_offset + 12]
                .copy_from_slice(&mismatched_count.to_le_bytes());
        })
        .await;

        assert_owned_layout_rejection(
            result,
            "TIFF tile-index descriptor count mismatch: TileOffsets has 2041930 entries, TileByteCounts has 2041929",
        );
        assert_eq!(counters, (0, 3, 0, 404, 404));
    }

    #[tokio::test]
    async fn owned_ifd_walker_rejects_ifd_entry_count_ceiling() {
        let (result, counters) = mutated_planetary_layout(838, |bytes| {
            let ifd_offset = bigtiff_ifd_offset(bytes);
            let count = MAX_REMOTE_IFD_ENTRIES
                .checked_add(1)
                .expect("entry ceiling increment should fit");
            bytes[ifd_offset..ifd_offset + 8].copy_from_slice(&count.to_le_bytes());
        })
        .await;

        assert_owned_layout_rejection(
            result,
            "TIFF IFD entry count 4097 exceeds parser ceiling 4096",
        );
        assert_eq!(counters, (0, 2, 0, 24, 24));
    }

    #[tokio::test]
    async fn owned_ifd_walker_rejects_ifd_entry_byte_ceiling() {
        let (result, counters) = mutated_planetary_layout(838, |bytes| {
            let ifd_offset = bigtiff_ifd_offset(bytes);
            let count = MAX_REMOTE_IFD_ENTRY_BYTES
                .checked_div(20)
                .and_then(|value| value.checked_add(1))
                .expect("entry-byte ceiling count should fit");
            bytes[ifd_offset..ifd_offset + 8].copy_from_slice(&count.to_le_bytes());
        })
        .await;

        assert_owned_layout_rejection(
            result,
            "TIFF IFD entry bytes 65540 exceeds parser ceiling 65536",
        );
        assert_eq!(counters, (0, 2, 0, 24, 24));
    }

    #[tokio::test]
    async fn owned_ifd_walker_rejects_metadata_value_byte_ceiling() {
        let (result, counters) = mutated_planetary_layout(838, |bytes| {
            let entry_offset = bigtiff_entry_offset(bytes, 33_922);
            let count = MAX_REMOTE_METADATA_VALUE_BYTES
                .checked_div(8)
                .and_then(|value| value.checked_add(1))
                .expect("metadata-value ceiling count should fit");
            bytes[entry_offset + 4..entry_offset + 12].copy_from_slice(&count.to_le_bytes());
        })
        .await;

        assert_owned_layout_rejection(
            result,
            "TIFF metadata value bytes 65544 exceeds parser ceiling 65536",
        );
        assert_eq!(counters, (0, 3, 0, 404, 404));
    }

    #[tokio::test]
    async fn remote_ascii_rejects_count_above_ceiling_with_typed_error() {
        let (result, counters) = mutated_planetary_layout(PLANETARY_FILE_LEN, |bytes| {
            let entry_offset = bigtiff_entry_offset(bytes, 42_113);
            bytes[entry_offset + 4..entry_offset + 12].copy_from_slice(&257_u64.to_le_bytes());
        })
        .await;

        match result {
            Err(CacheError::RemoteTiffAsciiTooLong {
                path,
                tag,
                length,
                limit,
            }) => {
                assert_eq!(path, ObjectPath::from("mutated.tif"));
                assert_eq!(tag, 42_113);
                assert_eq!(length, 257);
                assert_eq!(limit, 256);
            }
            other => panic!("expected RemoteTiffAsciiTooLong, got {other:?}"),
        }
        assert_eq!(counters, (0, 3, 0, 404, 404));
    }

    #[tokio::test]
    async fn remote_ascii_rejects_out_of_object_range_before_fetch() {
        let offset = PLANETARY_FILE_LEN - 4;
        let (result, counters) = mutated_planetary_layout(PLANETARY_FILE_LEN, |bytes| {
            let entry_offset = bigtiff_entry_offset(bytes, 42_113);
            bytes[entry_offset + 4..entry_offset + 12].copy_from_slice(&9_u64.to_le_bytes());
            bytes[entry_offset + 12..entry_offset + 20].copy_from_slice(&offset.to_le_bytes());
        })
        .await;

        assert_owned_layout_rejection(
            result,
            "TIFF range 24507155..24507164 exceeds object size 24507159",
        );
        assert_eq!(counters, (0, 3, 0, 404, 404));
    }

    #[tokio::test]
    async fn remote_ascii_reads_one_bounded_out_of_line_range() {
        let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/parity/tiny-with-aux-d8-projected-grass");
        for (relative_path, expected_nodata, declared_bytes) in [
            ("aux/d8/projected/flow_dir.tif", "-128", 5_usize),
            ("aux/d8/projected/flow_acc.tif", "-2147483648", 12_usize),
        ] {
            let local_store = LocalFileSystem::new_with_prefix(&fixture_root)
                .expect("projected GRASS fixture object store should be rooted");
            let store = CogFixtureCountingStore::new(Arc::new(local_store));
            let path = ObjectPath::from(relative_path);
            let object_size = fs::metadata(fixture_root.join(relative_path))
                .expect("projected GRASS TIFF metadata should be readable")
                .len();

            let layout = read_remote_layout(&store, &path, object_size)
                .await
                .expect("out-of-line ASCII metadata should parse");

            assert_eq!(layout.nodata, expected_nodata);
            assert_eq!(store.get_range_calls(), 4);
            assert_eq!(store.get_ranges_calls(), 1);
            assert_eq!(store.requested_range_bytes(), layout.bytes_read as u64);
            assert_eq!(store.consumed_range_bytes(), layout.bytes_read as u64);
            assert_eq!(
                store.get_range_lengths().last(),
                Some(&(declared_bytes as u64))
            );
            assert_eq!(layout.tile_offsets.byte_extent().unwrap(), None);
            assert_eq!(layout.tile_byte_counts.byte_extent().unwrap(), None);
        }
    }

    #[tokio::test]
    async fn owned_ifd_walker_rejects_wrong_model_pixel_scale_field_type() {
        let (result, counters) = mutated_planetary_layout(PLANETARY_FILE_LEN, |bytes| {
            let entry_offset = bigtiff_entry_offset(bytes, 33_550);
            bytes[entry_offset + 2..entry_offset + 4].copy_from_slice(&4_u16.to_le_bytes());
        })
        .await;

        assert_owned_layout_rejection(
            result,
            "TIFF tag 33550 must use DOUBLE field type 12, got 4",
        );
        assert_eq!(counters, (0, 3, 0, 404, 404));
    }

    #[tokio::test]
    async fn owned_ifd_walker_rejects_wrong_model_pixel_scale_count() {
        let (result, counters) = mutated_planetary_layout(PLANETARY_FILE_LEN, |bytes| {
            let entry_offset = bigtiff_entry_offset(bytes, 33_550);
            bytes[entry_offset + 4..entry_offset + 12].copy_from_slice(&2_u64.to_le_bytes());
        })
        .await;

        assert_owned_layout_rejection(
            result,
            "TIFF tag 33550 must contain exactly 3 DOUBLE values, got 2",
        );
        assert_eq!(counters, (0, 3, 0, 404, 404));
    }

    #[tokio::test]
    async fn covered_index_resolution_rejects_short_response() {
        let fixtures = fixtures();
        let oracle = CrossTileFixtureOracle::new();
        let local_store = LocalFileSystem::new_with_prefix(fixtures.temp_dir.path()).unwrap();
        let store = CogFixtureCountingStore::with_short_response(Arc::new(local_store), 8);
        let request =
            RasterWindowRequest::new(RasterKind::FlowDir, oracle.one_tile_planetary_bbox());
        let path = fixtures.planetary_object_path.clone();

        assert_unsupported_reason(
            prepare_window(&store, &path, &request).await,
            &path,
            "TIFF tile-index tag 324 entry 2039838 returned 7 bytes, expected 8",
        );
    }

    #[tokio::test]
    async fn compressed_chunk_fetch_rejects_short_response() {
        let fixtures = fixtures();
        let oracle = CrossTileFixtureOracle::new();
        let temp_dir = tempfile::TempDir::new().unwrap();
        let path = ObjectPath::from("short-compressed.tif");
        let local_path = temp_dir.path().join(path.as_ref());
        fs::copy(
            fixtures
                .temp_dir
                .path()
                .join(fixtures.planetary_object_path.as_ref()),
            &local_path,
        )
        .unwrap();
        let mut file = File::options().write(true).open(&local_path).unwrap();
        file.seek(SeekFrom::Start(24_498_790)).unwrap();
        write_u32(&mut file, 16).unwrap();
        drop(file);
        let local_store = LocalFileSystem::new_with_prefix(temp_dir.path()).unwrap();
        let store = CogFixtureCountingStore::with_short_response(Arc::new(local_store), 16);
        let request =
            RasterWindowRequest::new(RasterKind::FlowDir, oracle.one_tile_planetary_bbox());
        let prepared = prepare_window(&store, &path, &request).await.unwrap();
        let output = temp_dir.path().join("short-window.tif");

        assert_unsupported_reason(
            fetch_window_to_path(&store, &path, prepared, &output).await,
            &path,
            "TIFF compressed chunk 2039838 returned 15 bytes, expected 16",
        );
    }

    #[tokio::test]
    async fn known_value_deflate_chunk_with_predictor_one_decodes_without_differencing() {
        let fixtures = fixtures();
        let local_store = LocalFileSystem::new_with_prefix(fixtures.temp_dir.path())
            .expect("fixture object store should be rooted");
        let request = RasterWindowRequest::new(
            RasterKind::FlowDir,
            Rect::new(
                coord! { x: 1_069_057.0, y: -499_202.0 },
                coord! { x: 1_069_063.0, y: -499_201.0 },
            ),
        );
        let prepared = prepare_window(&local_store, &fixtures.planetary_object_path, &request)
            .await
            .expect("planetary window should prepare");
        let output = fixtures.temp_dir.path().join("known-u8-window.tif");
        fetch_window_to_path(
            &local_store,
            &fixtures.planetary_object_path,
            prepared,
            &output,
        )
        .await
        .expect("planetary window should materialize");
        let mut decoder = Decoder::new(File::open(output).unwrap()).unwrap();
        let DecodingResult::U8(decoded) = decoder.read_image().unwrap() else {
            panic!("known U8 window should decode as U8");
        };

        assert_eq!(&decoded[..8], [1, 2, 3, 4, 5, 6, 7, 8]);
        assert_ne!(&decoded[..8], [1, 3, 6, 10, 15, 21, 28, 36]);
    }

    #[test]
    fn decode_window_preserves_declared_nodata_for_unwritten_spanned_tile() {
        let mut meta = metadata();
        meta.width = 4;
        meta.height = 2;
        meta.tile_width = 2;
        meta.tile_height = 2;
        meta.sample_type = CogSampleType::U8;
        meta.compression = 8;
        meta.predictor = 1;
        meta.nodata = "255".to_string();
        let window = RasterPixelWindow {
            col_off: 0,
            row_off: 0,
            width: 4,
            height: 2,
        };
        let compressed = compress_tile(&[1_u8, 2, 4, 8]).unwrap();
        let compressed_len = u64::try_from(compressed.len()).unwrap();
        let plan = ResolvedTilePlan {
            object_size: compressed_len,
            tiles: vec![ResolvedTile {
                index: 0,
                range: 0..compressed_len,
            }],
            compressed_bytes: compressed_len,
        };

        let WindowData::U8(decoded) = decode_window(
            &[compressed],
            &meta,
            window,
            &plan,
            &ObjectPath::from("unwritten-direction-window.tif"),
        )
        .unwrap() else {
            panic!("U8 metadata should produce a U8 window");
        };

        assert_eq!(decoded, vec![1, 2, 255, 255, 4, 8, 255, 255]);
    }

    #[test]
    fn decode_window_prefills_i8_nodata_as_stored_byte() {
        let mut meta = metadata();
        meta.width = 1;
        meta.height = 1;
        meta.tile_width = 1;
        meta.tile_height = 1;
        meta.sample_type = CogSampleType::I8;
        meta.nodata = "-1".to_string();
        let window = RasterPixelWindow {
            col_off: 0,
            row_off: 0,
            width: 1,
            height: 1,
        };
        let plan = ResolvedTilePlan {
            object_size: 0,
            tiles: Vec::new(),
            compressed_bytes: 0,
        };
        let compressed: [&[u8]; 0] = [];

        let WindowData::U8(decoded) = decode_window(
            &compressed,
            &meta,
            window,
            &plan,
            &ObjectPath::from("i8-nodata-window.tif"),
        )
        .unwrap() else {
            panic!("I8 metadata should produce a U8 window");
        };

        assert_eq!(decoded, vec![255]);
    }

    #[test]
    fn decode_window_rejects_unrepresentable_direction_nodata() {
        fn assert_rejected(sample_type: CogSampleType, declared: &str, expected_reason: &str) {
            let mut meta = metadata();
            meta.width = 1;
            meta.height = 1;
            meta.tile_width = 1;
            meta.tile_height = 1;
            meta.sample_type = sample_type;
            meta.nodata = declared.to_string();
            let window = RasterPixelWindow {
                col_off: 0,
                row_off: 0,
                width: 1,
                height: 1,
            };
            let plan = ResolvedTilePlan {
                object_size: 0,
                tiles: Vec::new(),
                compressed_bytes: 0,
            };
            let compressed: [&[u8]; 0] = [];
            let path = ObjectPath::from("invalid-direction-nodata.tif");

            match decode_window(&compressed, &meta, window, &plan, &path) {
                Err(CacheError::UnsupportedCog {
                    path: error_path,
                    reason,
                }) => {
                    assert_eq!(error_path, path);
                    assert_eq!(reason, expected_reason);
                }
                Err(other) => panic!("expected UnsupportedCog, got {other:?}"),
                Ok(_) => panic!("unrepresentable direction nodata should fail"),
            }
        }

        assert_rejected(
            CogSampleType::U8,
            "not-a-byte",
            "declared U8 nodata \"not-a-byte\" is not representable as u8",
        );
        assert_rejected(
            CogSampleType::I8,
            "128",
            "declared I8 nodata \"128\" is not representable as i8",
        );
    }

    #[tokio::test]
    async fn flow_acc_predictor_one_decodes_known_values_without_differencing() {
        let fixtures = fixtures();
        let local_store = LocalFileSystem::new_with_prefix(fixtures.temp_dir.path())
            .expect("fixture object store should be rooted");
        let request = RasterWindowRequest::new(
            RasterKind::FlowAcc,
            Rect::new(coord! { x: 0.0, y: -1.0 }, coord! { x: 3.0, y: 0.0 }),
        );
        let prepared = prepare_window(&local_store, &fixtures.flow_acc_object_path, &request)
            .await
            .expect("FlowAcc window should prepare");
        let output = fixtures.temp_dir.path().join("known-f32-window.tif");
        fetch_window_to_path(
            &local_store,
            &fixtures.flow_acc_object_path,
            prepared,
            &output,
        )
        .await
        .expect("FlowAcc window should materialize");
        let mut decoder = Decoder::new(File::open(output).unwrap()).unwrap();
        let DecodingResult::F32(decoded) = decoder.read_image().unwrap() else {
            panic!("known F32 window should decode as F32");
        };

        // Parsed nodata is -1, so the remote F32 arm copies these samples verbatim.
        assert_eq!(&decoded[..3], [1_000.0, 1_001.0, 1_002.0]);
        let stored_bits = [
            decoded[0].to_bits(),
            decoded[1].to_bits(),
            decoded[2].to_bits(),
        ];
        assert_eq!(stored_bits, [0x447a0000, 0x447a4000, 0x447a8000]);
        assert_ne!(stored_bits, [0x447a0000, 0x88f44000, 0xcd6ec000]);
        assert_ne!(stored_bits, [0x00c08040, 0x00c08040, 0x7a3afaba]);
    }

    #[test]
    fn prototype_decode_rejects_unsupported_predictor() {
        let compressed = compress_tile(&vec![0_u8; 512 * 512])
            .expect("valid full-tile zlib payload should be created");
        let mut meta = metadata();
        meta.width = 512;
        meta.height = 512;
        meta.sample_type = CogSampleType::U8;
        meta.predictor = 99;
        let path = ObjectPath::from("unsupported-predictor.tif");

        let error = decode_owned_chunk(&compressed, &meta, 0, &path)
            .expect_err("unsupported predictor should fail");
        match error {
            CacheError::UnsupportedCog {
                path: error_path,
                reason,
            } => {
                assert_eq!(error_path, path);
                assert_eq!(reason, "unsupported TIFF predictor 99 for U8");
            }
            other => panic!("expected UnsupportedCog, got {other:?}"),
        }
    }

    fn assert_unsupported_reason(
        result: Result<impl std::fmt::Debug, CacheError>,
        path: &ObjectPath,
        expected: &str,
    ) {
        match result {
            Err(CacheError::UnsupportedCog {
                path: error_path,
                reason,
            }) => {
                assert_eq!(&error_path, path);
                assert_eq!(reason, expected);
            }
            other => panic!("expected UnsupportedCog, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn owned_decode_rejects_wrong_decoded_length() {
        let fixtures = fixtures();
        let oracle = CrossTileFixtureOracle::new();
        let temp_dir = tempfile::TempDir::new().unwrap();
        let path = ObjectPath::from("wrong-length.tif");
        let local_path = temp_dir.path().join(path.as_ref());
        fs::copy(
            fixtures
                .temp_dir
                .path()
                .join(fixtures.planetary_object_path.as_ref()),
            &local_path,
        )
        .unwrap();
        let compressed = compress_tile(&vec![0_u8; 262_143]).unwrap();
        let mut file = File::options().write(true).open(&local_path).unwrap();
        file.seek(SeekFrom::Start(668)).unwrap();
        file.write_all(&compressed).unwrap();
        file.seek(SeekFrom::Start(24_498_790)).unwrap();
        write_u32(&mut file, u32::try_from(compressed.len()).unwrap()).unwrap();
        drop(file);
        let store = LocalFileSystem::new_with_prefix(temp_dir.path()).unwrap();
        let request =
            RasterWindowRequest::new(RasterKind::FlowDir, oracle.one_tile_planetary_bbox());
        let prepared = prepare_window(&store, &path, &request).await.unwrap();
        let output = temp_dir.path().join("wrong-length-window.tif");
        assert_unsupported_reason(
            fetch_window_to_path(&store, &path, prepared, &output).await,
            &path,
            "TIFF tile 2039838 decoded 262143 bytes, expected 262144",
        );
    }

    #[test]
    fn owned_decode_preserves_all_sample_types() {
        let path = ObjectPath::from("all-samples.tif");
        let mut meta = metadata();
        meta.width = 3;
        meta.height = 1;
        meta.tile_width = 3;
        meta.tile_height = 1;
        meta.nodata = "-1".to_string();

        meta.sample_type = CogSampleType::U8;
        meta.predictor = 2;
        let compressed = compress_tile(&[1, 1, 1]).unwrap();
        assert_eq!(
            decode_owned_chunk(&compressed, &meta, 0, &path).unwrap(),
            OwnedTileData::U8(vec![1, 2, 3])
        );

        meta.sample_type = CogSampleType::I8;
        let compressed = compress_tile(&[0xff, 0xff, 0xff]).unwrap();
        assert_eq!(
            decode_owned_chunk(&compressed, &meta, 0, &path).unwrap(),
            OwnedTileData::U8(vec![255, 254, 253])
        );

        meta.sample_type = CogSampleType::I32;
        let encoded = [1_i32, 1, 1]
            .into_iter()
            .flat_map(i32::to_le_bytes)
            .collect::<Vec<_>>();
        let compressed = compress_tile(&encoded).unwrap();
        assert_eq!(
            decode_owned_chunk(&compressed, &meta, 0, &path).unwrap(),
            OwnedTileData::F32(vec![1.0, 2.0, 3.0])
        );

        meta.sample_type = CogSampleType::F32;
        meta.predictor = 3;
        let encoded = [
            0x3f, 0x01, 0x00, 0x40, 0x80, 0x40, 0xc0, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        let compressed = compress_tile(&encoded).unwrap();
        assert_eq!(
            decode_owned_chunk(&compressed, &meta, 0, &path).unwrap(),
            OwnedTileData::F32(vec![1.0, 2.0, 3.0])
        );
    }

    #[test]
    fn owned_decode_predictor_three_uses_padded_plane_stride_on_edge_tile() {
        let path = ObjectPath::from("predictor-three-padded-edge.tif");
        let mut meta = metadata();
        meta.width = 5;
        meta.height = 1;
        meta.tile_width = 3;
        meta.tile_height = 1;
        meta.sample_type = CogSampleType::F32;
        meta.predictor = 3;
        let encoded = [
            0x3f, 0x01, 0x00, 0x40, 0x80, 0x40, 0xc0, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        let compressed = compress_tile(&encoded).unwrap();

        assert_eq!(
            decode_owned_chunk(&compressed, &meta, 1, &path).unwrap(),
            OwnedTileData::F32(vec![1.0, 2.0])
        );
    }

    #[test]
    fn decoded_chunk_ceiling_is_fixed_with_defended_headroom() {
        let decoded_chunk_ceiling = MAX_DECODED_CHUNK_BYTES;

        assert_eq!(decoded_chunk_ceiling, 8_388_608_u64);
    }

    #[tokio::test]
    async fn cache_route_places_four_tiles_and_clips_raster_edges() {
        let fixtures = fixtures();
        let oracle = CrossTileFixtureOracle::new();
        let store = LocalFileSystem::new_with_prefix(fixtures.temp_dir.path())
            .expect("fixture object store should be rooted");
        let cache_temp = tempfile::TempDir::new().expect("cache temp directory should be created");
        let cache = crate::raster_cache::RemoteRasterCache::new(cache_temp.path().to_path_buf());

        let flow_dir_request =
            RasterWindowRequest::new(RasterKind::FlowDir, oracle.flow_dir_bbox());
        let prepared = prepare_window(&store, &fixtures.planetary_object_path, &flow_dir_request)
            .await
            .expect("FlowDir production route should prepare the four-tile window");
        let observations = ceiling_observations(&prepared, 1_u64);
        assert_eq!(observations, oracle.flow_dir_ceiling_observations());
        assert_required_ceiling_margins(observations, oracle.flow_dir_required_ceiling_margins());
        let localized = cache
            .get_or_fetch_window(
                &store,
                &fixtures.planetary_object_path,
                &flow_dir_request,
                "test-fabric",
                "0.1.0",
            )
            .await
            .expect("FlowDir cache route should localize the four-tile window");
        assert!(localized.path().exists());
        assert_eq!(localized.tile_count(), 4);
        assert_eq!(localized.window_pixels(), 755_200);
        let mut decoder = Decoder::new(File::open(localized.path()).unwrap()).unwrap();
        assert_eq!(decoder.dimensions().unwrap(), (944, 800));
        let DecodingResult::U8(decoded) = decoder.read_image().unwrap() else {
            panic!("FlowDir localized window should decode as U8");
        };
        assert_eq!(decoded.len(), 755_200);
        let flow_dir_window = oracle.flow_dir_window();
        for row in 0..flow_dir_window.height {
            for col in 0..flow_dir_window.width {
                let index = usize::try_from(row * 944 + col).unwrap();
                assert_eq!(
                    decoded[index],
                    oracle.expected_u8(row, col),
                    "FlowDir sample mismatch at output row {row}, column {col}"
                );
            }
        }

        let flow_acc_request =
            RasterWindowRequest::new(RasterKind::FlowAcc, oracle.flow_acc_bbox());
        let prepared = prepare_window(&store, &fixtures.flow_acc_object_path, &flow_acc_request)
            .await
            .expect("FlowAcc production route should prepare the four-tile window");
        let observations = ceiling_observations(&prepared, 4_u64);
        assert_eq!(observations, oracle.flow_acc_ceiling_observations());
        assert_required_ceiling_margins(observations, oracle.flow_acc_required_ceiling_margins());
        let localized = cache
            .get_or_fetch_window(
                &store,
                &fixtures.flow_acc_object_path,
                &flow_acc_request,
                "test-fabric",
                "0.1.0",
            )
            .await
            .expect("FlowAcc cache route should localize the four-tile window");
        assert!(localized.path().exists());
        assert_eq!(localized.tile_count(), 4);
        assert_eq!(localized.window_pixels(), 755_200);
        let mut decoder = Decoder::new(File::open(localized.path()).unwrap()).unwrap();
        assert_eq!(decoder.dimensions().unwrap(), (944, 800));
        let DecodingResult::F32(decoded) = decoder.read_image().unwrap() else {
            panic!("FlowAcc localized window should decode as F32");
        };
        assert_eq!(decoded.len(), 755_200);
        let flow_acc_window = oracle.flow_acc_window();
        for row in 0..flow_acc_window.height {
            for col in 0..flow_acc_window.width {
                let index = usize::try_from(row * 944 + col).unwrap();
                assert_eq!(
                    decoded[index],
                    oracle.expected_f32(row, col),
                    "FlowAcc sample mismatch at output row {row}, column {col}"
                );
            }
        }
    }

    #[test]
    fn tile_plan_rejects_planned_tile_ceiling() {
        let path = ObjectPath::from("tile-ceiling.tif");
        let mut meta = metadata();
        meta.width = 512;
        meta.height = 33_554_433;
        let window = RasterPixelWindow {
            col_off: 0,
            row_off: 0,
            width: meta.width,
            height: meta.height,
        };
        let over = MAX_PLANNED_TILE_COUNT.checked_add(1).unwrap();
        assert_unsupported_reason(
            TilePlan::for_window(&meta, window, &path),
            &path,
            &format!(
                "TIFF planned tile count {over} exceeds window ceiling {MAX_PLANNED_TILE_COUNT}"
            ),
        );
    }

    #[test]
    fn covered_chunks_reject_individual_compressed_ceiling() {
        let path = ObjectPath::from("individual-ceiling.tif");
        let over = MAX_COMPRESSED_CHUNK_BYTES.checked_add(1).unwrap();
        let plan = ResolvedTilePlan {
            object_size: over,
            tiles: vec![ResolvedTile {
                index: 0,
                range: 0..over,
            }],
            compressed_bytes: over,
        };
        assert_unsupported_reason(
            validate_compressed_ranges(&path, &plan),
            &path,
            &format!(
                "TIFF compressed chunk 0 bytes {over} exceeds window ceiling {MAX_COMPRESSED_CHUNK_BYTES}"
            ),
        );
    }

    #[test]
    fn covered_chunks_reject_aggregate_compressed_ceiling() {
        let path = ObjectPath::from("aggregate-ceiling.tif");
        let over = MAX_COVERED_CHUNK_BYTES.checked_add(1).unwrap();
        let mut start = 0_u64;
        let mut tiles = Vec::new();
        while start < over {
            let end = min(start.checked_add(MAX_COMPRESSED_CHUNK_BYTES).unwrap(), over);
            tiles.push(ResolvedTile {
                index: u32::try_from(tiles.len()).unwrap(),
                range: start..end,
            });
            start = end;
        }
        let plan = ResolvedTilePlan {
            object_size: over,
            tiles,
            compressed_bytes: over,
        };
        assert_unsupported_reason(
            validate_compressed_ranges(&path, &plan),
            &path,
            &format!(
                "TIFF covered compressed bytes {over} exceeds window ceiling {MAX_COVERED_CHUNK_BYTES}"
            ),
        );
    }

    #[test]
    fn window_allocation_rejects_byte_ceiling() {
        let path = ObjectPath::from("window-ceiling.tif");
        let over = MAX_WINDOW_ALLOCATION_BYTES.checked_add(1).unwrap();
        assert_eq!(25_u64 * 42_949_673_u64, over);
        assert_unsupported_reason(
            window_allocation_len(&path, 25, 42_949_673, 1),
            &path,
            &format!(
                "TIFF window allocation bytes {over} exceeds window ceiling {MAX_WINDOW_ALLOCATION_BYTES}"
            ),
        );
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
            index: CogIndex::Remote {
                tile_offsets: IndexDescriptor {
                    field_type: 16,
                    element_width: 8,
                    count: 8,
                    storage: IndexStorage::OutOfLine(1_000),
                },
                tile_byte_counts: IndexDescriptor {
                    field_type: 4,
                    element_width: 4,
                    count: 8,
                    storage: IndexStorage::OutOfLine(2_000),
                },
            },
        }
    }

    #[test]
    fn validate_merit_layout_accepts_predictor_one_u8() {
        let mut meta = metadata();
        meta.width = 512;
        meta.height = 512;
        meta.index = CogIndex::Remote {
            tile_offsets: IndexDescriptor {
                field_type: 16,
                element_width: 8,
                count: 1,
                storage: IndexStorage::InlineScalar(704),
            },
            tile_byte_counts: IndexDescriptor {
                field_type: 4,
                element_width: 4,
                count: 1,
                storage: IndexStorage::InlineScalar(1),
            },
        };
        meta.predictor = 1;

        meta.sample_type = CogSampleType::U8;
        validate_merit_layout(
            &meta,
            RasterKind::FlowDir,
            &ObjectPath::from("predictor-one.tif"),
        )
        .expect("predictor 1 U8 should be accepted");
        meta.sample_type = CogSampleType::I8;
        validate_merit_layout(
            &meta,
            RasterKind::FlowDir,
            &ObjectPath::from("predictor-one.tif"),
        )
        .expect("predictor 1 I8 should be accepted");
        meta.sample_type = CogSampleType::I32;
        validate_merit_layout(
            &meta,
            RasterKind::FlowAcc,
            &ObjectPath::from("predictor-one.tif"),
        )
        .expect("predictor 1 I32 should be accepted");
    }

    #[test]
    fn validate_merit_layout_accepts_predictor_one_f32() {
        let mut meta = metadata();
        meta.width = 1536;
        meta.height = 512;
        meta.nodata = "-1".to_string();
        meta.sample_type = CogSampleType::F32;
        meta.predictor = 1;
        meta.index = CogIndex::Remote {
            tile_offsets: IndexDescriptor {
                field_type: 16,
                element_width: 8,
                count: 3,
                storage: IndexStorage::OutOfLine(704),
            },
            tile_byte_counts: IndexDescriptor {
                field_type: 4,
                element_width: 4,
                count: 3,
                storage: IndexStorage::OutOfLine(728),
            },
        };

        validate_merit_layout(
            &meta,
            RasterKind::FlowAcc,
            &ObjectPath::from("predictor-one.tif"),
        )
        .expect("predictor 1 F32 should be accepted");
    }

    #[test]
    fn validate_merit_layout_rejects_f32_predictor_two() {
        let mut meta = metadata();
        meta.width = 512;
        meta.height = 512;
        meta.nodata = "-1".to_string();
        meta.sample_type = CogSampleType::F32;
        meta.predictor = 2;
        meta.index = CogIndex::Remote {
            tile_offsets: IndexDescriptor {
                field_type: 16,
                element_width: 8,
                count: 1,
                storage: IndexStorage::InlineScalar(704),
            },
            tile_byte_counts: IndexDescriptor {
                field_type: 4,
                element_width: 4,
                count: 1,
                storage: IndexStorage::InlineScalar(1),
            },
        };

        let path = ObjectPath::from("predictor-two.tif");
        let error = validate_merit_layout(&meta, RasterKind::FlowAcc, &path)
            .expect_err("predictor 2 F32 should be rejected");
        match error {
            CacheError::UnsupportedCog {
                path: error_path,
                reason,
            } => {
                assert_eq!(error_path, path);
                assert_eq!(reason, "F32 supports TIFF predictors 1 or 3, got 2");
            }
            other => panic!("expected UnsupportedCog, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn cross_tile_fixture_oracle_locks_observation_geometry() {
        let fixtures = fixtures();
        let oracle = CrossTileFixtureOracle::new();
        assert_eq!(
            oracle.flow_dir_bbox(),
            Rect::new(
                coord! { x: 1_069_057.0, y: -499_999.0 },
                coord! { x: 1_069_999.0, y: -499_201.0 },
            )
        );
        assert_eq!(
            oracle.flow_dir_window(),
            RasterPixelWindow {
                col_off: 1_069_056,
                row_off: 499_200,
                width: 944,
                height: 800,
            }
        );
        assert_eq!(
            oracle.flow_dir_tile_indices(),
            [2_039_838, 2_039_839, 2_041_928, 2_041_929]
        );
        assert_eq!(
            oracle.flow_acc_bbox(),
            Rect::new(coord! { x: 1.0, y: -799.0 }, coord! { x: 943.0, y: -1.0 },)
        );
        assert_eq!(
            oracle.flow_acc_window(),
            RasterPixelWindow {
                col_off: 0,
                row_off: 0,
                width: 944,
                height: 800,
            }
        );
        assert_eq!(oracle.flow_acc_tile_indices(), [0, 1, 2, 3]);
        assert_eq!(
            oracle.live_tile_dimensions(),
            [(512, 512), (432, 512), (512, 288), (432, 288)]
        );

        let store = LocalFileSystem::new_with_prefix(fixtures.temp_dir.path())
            .expect("fixture object store should be rooted");
        let flow_dir_request =
            RasterWindowRequest::new(RasterKind::FlowDir, oracle.flow_dir_bbox());
        let flow_dir = prepare_window(&store, &fixtures.planetary_object_path, &flow_dir_request)
            .await
            .expect("four-tile FlowDir observation should prepare");
        let flow_acc_request =
            RasterWindowRequest::new(RasterKind::FlowAcc, oracle.flow_acc_bbox());
        let flow_acc = prepare_window(&store, &fixtures.flow_acc_object_path, &flow_acc_request)
            .await
            .expect("four-tile FlowAcc observation should prepare");

        assert_eq!(flow_dir.window, oracle.flow_dir_window());
        assert_eq!(
            flow_dir
                .plan
                .tiles
                .iter()
                .map(|tile| tile.index)
                .collect::<Vec<_>>(),
            oracle.flow_dir_tile_indices()
        );
        assert_eq!(flow_acc.window, oracle.flow_acc_window());
        assert_eq!(
            flow_acc
                .plan
                .tiles
                .iter()
                .map(|tile| tile.index)
                .collect::<Vec<_>>(),
            oracle.flow_acc_tile_indices()
        );
        assert_eq!(
            (
                flow_dir.metadata.width,
                flow_dir.metadata.height,
                flow_dir.metadata.tiles_across(),
                flow_dir.metadata.tiles_down(),
            ),
            (1_070_000, 500_000, 2_090, 977)
        );
        assert_eq!(
            (
                flow_acc.metadata.width,
                flow_acc.metadata.height,
                flow_acc.metadata.tiles_across(),
                flow_acc.metadata.tiles_down(),
            ),
            (944, 800, 2, 2)
        );
        assert_eq!((flow_dir.window.width, flow_dir.window.height), (944, 800));
        assert_eq!((flow_acc.window.width, flow_acc.window.height), (944, 800));
        for prepared in [&flow_dir, &flow_acc] {
            let right_width = prepared.window.width.saturating_sub(512).min(512);
            let bottom_height = prepared.window.height.saturating_sub(512).min(512);
            let dimensions = [
                (
                    prepared.window.width.min(512),
                    prepared.window.height.min(512),
                ),
                (right_width, prepared.window.height.min(512)),
                (prepared.window.width.min(512), bottom_height),
                (right_width, bottom_height),
            ];
            assert_eq!(dimensions, oracle.live_tile_dimensions());
            assert_eq!(dimensions[0].0 + dimensions[1].0, prepared.window.width);
            assert_eq!(dimensions[0].1 + dimensions[2].1, prepared.window.height);
        }
    }

    #[test]
    fn planetary_fixture_populates_only_cross_tile_sparse_entries() {
        let fixtures = fixtures();
        let oracle = CrossTileFixtureOracle::new();
        let bytes = fs::read(
            fixtures
                .temp_dir
                .path()
                .join(fixtures.planetary_object_path.as_ref()),
        )
        .expect("planetary fixture should be readable");
        assert_eq!(bytes.len(), usize::try_from(PLANETARY_FILE_LEN).unwrap());
        assert_eq!(
            PLANETARY_TILE_INDICES.map(|index| 3_998 + 8 * u64::from(index)),
            [16_322_702, 16_322_710, 16_339_422, 16_339_430]
        );
        assert_eq!(
            PLANETARY_TILE_INDICES.map(|index| 16_339_438 + 4 * u64::from(index)),
            [24_498_790, 24_498_794, 24_507_150, 24_507_154]
        );

        let mut decoded_tiles = Vec::new();
        for (slot, ((index, expected_offset), expected_count)) in PLANETARY_TILE_INDICES
            .into_iter()
            .zip(PLANETARY_TILE_OFFSETS)
            .zip(PLANETARY_TILE_BYTE_COUNTS)
            .enumerate()
        {
            let offset_entry = usize::try_from(3_998 + 8 * u64::from(index)).unwrap();
            let count_entry = usize::try_from(16_339_438 + 4 * u64::from(index)).unwrap();
            let offset =
                u64::from_le_bytes(bytes[offset_entry..offset_entry + 8].try_into().unwrap());
            let count = u32::from_le_bytes(bytes[count_entry..count_entry + 4].try_into().unwrap());
            assert_eq!((offset, count), (expected_offset, expected_count));

            let payload_start = usize::try_from(offset).unwrap();
            let payload_end = payload_start + usize::try_from(count).unwrap();
            let decoded = prototype_decode(&bytes[payload_start..payload_end])
                .expect("planetary tile should inflate");
            assert_eq!(decoded.len(), 262_144);
            for local_row in 0..512 {
                for local_col in 0..512 {
                    let sample = decoded[usize::try_from(local_row * 512 + local_col).unwrap()];
                    let slot = u32::try_from(slot).unwrap();
                    let observation_row = (slot / 2) * 512 + local_row;
                    let observation_col = (slot % 2) * 512 + local_col;
                    assert_eq!(sample, oracle.expected_u8(observation_row, observation_col));
                    assert_ne!(sample, 255);
                }
            }
            decoded_tiles.push(decoded);
        }
        for left in 0..decoded_tiles.len() {
            for right in left + 1..decoded_tiles.len() {
                assert_ne!(decoded_tiles[left], decoded_tiles[right]);
            }
        }

        for unselected_index in [0_u32, 2_039_837, 2_041_927] {
            let offset_entry = usize::try_from(3_998 + 8 * u64::from(unselected_index)).unwrap();
            let count_entry =
                usize::try_from(16_339_438 + 4 * u64::from(unselected_index)).unwrap();
            assert_eq!(&bytes[offset_entry..offset_entry + 8], &[0; 8]);
            assert_eq!(&bytes[count_entry..count_entry + 4], &[0; 4]);
        }
        assert_eq!(3_998 + 8 * u64::from(PLANETARY_TILE_INDICES[3]), 16_339_430);
        assert_eq!(
            16_339_438 + 4 * u64::from(PLANETARY_TILE_INDICES[3]),
            24_507_154
        );
        assert_eq!(24_507_154 + 4, PLANETARY_INDEX_END);
        assert_eq!(bytes[usize::try_from(PLANETARY_INDEX_END).unwrap()], 0);
    }

    #[test]
    fn flow_acc_fixture_indexes_are_genuinely_out_of_line() {
        let fixtures = fixtures();
        let oracle = CrossTileFixtureOracle::new();
        let bytes = fs::read(fixtures.temp_dir.path().join("flow_acc.tif"))
            .expect("FlowAcc fixture should be readable");
        assert_eq!(bytes.len(), usize::try_from(FLOW_ACC_FILE_LEN).unwrap());
        let object_size = u64::try_from(bytes.len()).expect("fixture length should fit u64");
        let decode_entry = |slot: &[u8]| IfdEntry {
            tag: u16::from_le_bytes(slot[0..2].try_into().unwrap()),
            field_type: u16::from_le_bytes(slot[2..4].try_into().unwrap()),
            count: u64::from_le_bytes(slot[4..12].try_into().unwrap()),
            value: u64::from_le_bytes(slot[12..20].try_into().unwrap()),
        };
        let offsets_entry = decode_entry(&bytes[468..488]);
        let counts_entry = decode_entry(&bytes[488..508]);

        assert_eq!(
            (
                offsets_entry.tag,
                offsets_entry.field_type,
                offsets_entry.count,
                offsets_entry.value
            ),
            (324, 16, 4, 668)
        );
        assert_eq!(
            (
                counts_entry.tag,
                counts_entry.field_type,
                counts_entry.count,
                counts_entry.value
            ),
            (325, 4, 4, 700)
        );

        let path = ObjectPath::from("flow_acc.tif");
        let offsets =
            remote_descriptor(&path, TiffFormat::BigTiff, offsets_entry, object_size).unwrap();
        let counts =
            remote_descriptor(&path, TiffFormat::BigTiff, counts_entry, object_size).unwrap();
        assert_eq!(
            offsets,
            IndexDescriptor {
                field_type: 16,
                element_width: 8,
                count: 4,
                storage: IndexStorage::OutOfLine(668),
            }
        );
        assert_eq!(offsets.byte_extent().unwrap(), Some(668..700));
        assert_eq!(offsets.count * offsets.element_width, 32);
        assert!(offsets.count * offsets.element_width > 8);
        assert_eq!(
            counts,
            IndexDescriptor {
                field_type: 4,
                element_width: 4,
                count: 4,
                storage: IndexStorage::OutOfLine(700),
            }
        );
        assert_eq!(counts.byte_extent().unwrap(), Some(700..716));
        assert_eq!(counts.count * counts.element_width, 16);
        assert!(counts.count * counts.element_width > 8);
        assert!(!matches!(offsets.storage, IndexStorage::InlineScalar(_)));
        assert!(!matches!(counts.storage, IndexStorage::InlineScalar(_)));

        let tile_offsets = (0..4)
            .map(|index| {
                let start = 668 + index * 8;
                u64::from_le_bytes(bytes[start..start + 8].try_into().unwrap())
            })
            .collect::<Vec<_>>();
        let tile_counts = (0..4)
            .map(|index| {
                let start = 700 + index * 4;
                u32::from_le_bytes(bytes[start..start + 4].try_into().unwrap())
            })
            .collect::<Vec<_>>();
        assert_eq!(tile_offsets, FLOW_ACC_TILE_OFFSETS);
        assert_eq!(tile_counts, FLOW_ACC_TILE_BYTE_COUNTS);
        let expected_prefixes = [
            [1_000.0_f32, 1_001.0, 1_002.0],
            [2_000.0_f32, 2_001.0, 2_002.0],
            [3_000.0_f32, 3_001.0, 3_002.0],
            [4_000.0_f32, 4_001.0, 4_002.0],
        ];
        let mut decoded_tiles = Vec::new();
        for (slot, ((offset, count), expected)) in tile_offsets
            .iter()
            .zip(&tile_counts)
            .zip(expected_prefixes)
            .enumerate()
        {
            let start = usize::try_from(*offset).unwrap();
            let end = start + usize::try_from(*count).unwrap();
            let decoded =
                prototype_decode(&bytes[start..end]).expect("FlowAcc tile should inflate");
            assert_eq!(decoded.len(), 1_048_576);
            let prefix = [0, 4, 8]
                .map(|start| f32::from_le_bytes(decoded[start..start + 4].try_into().unwrap()));
            assert_eq!(prefix, expected);
            for local_row in 0..512 {
                for local_col in 0..512 {
                    let sample_start = usize::try_from((local_row * 512 + local_col) * 4).unwrap();
                    let sample = f32::from_le_bytes(
                        decoded[sample_start..sample_start + 4].try_into().unwrap(),
                    );
                    let slot = u32::try_from(slot).unwrap();
                    let observation_row = (slot / 2) * 512 + local_row;
                    let observation_col = (slot % 2) * 512 + local_col;
                    assert_eq!(
                        sample,
                        oracle.expected_f32(observation_row, observation_col)
                    );
                    assert!(sample.is_finite());
                    assert_ne!(sample, -1.0);
                }
            }
            decoded_tiles.push(decoded);
        }
        for left in 0..decoded_tiles.len() {
            for right in left + 1..decoded_tiles.len() {
                assert_ne!(decoded_tiles[left], decoded_tiles[right]);
            }
        }

        let tile_0 = &decoded_tiles[0];
        let stored_bits =
            [0, 4, 8].map(|start| u32::from_le_bytes(tile_0[start..start + 4].try_into().unwrap()));
        assert_eq!(stored_bits, [0x447a0000, 0x447a4000, 0x447a8000]);
        let predictor_2_bits = [0x447a0000, 0x88f44000, 0xcd6ec000];
        assert_ne!(predictor_2_bits, stored_bits);
        let predictor_3_bits = [0x00c08040, 0x00c08040, 0x7a3afaba];
        assert_ne!(predictor_3_bits, stored_bits);
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
    async fn planetary_extent_reads_without_materializing_tile_indexes() {
        let fixtures = fixtures();
        let local_store = LocalFileSystem::new_with_prefix(fixtures.temp_dir.path())
            .expect("fixture object store should be rooted");
        let inner: Arc<dyn ObjectStore> = Arc::new(local_store);
        let store = CogFixtureCountingStore::new(inner);

        let extent =
            read_remote_extent(&store as &dyn ObjectStore, &fixtures.planetary_object_path)
                .await
                .expect("planetary extent should parse without materializing tile indexes");

        assert_eq!(
            extent.rect(),
            Rect::new(
                coord! { x: 0.0, y: -500_000.0 },
                coord! { x: 1_070_000.0, y: 0.0 }
            )
        );
        assert_eq!(store.head_calls(), 1);
        assert_eq!(store.get_range_calls(), 3);
        assert_eq!(store.get_ranges_calls(), 1);
        assert_eq!(store.requested_range_bytes(), 476);
        assert_eq!(store.consumed_range_bytes(), 476);
        // TRANSITIONAL: [the M2 extent obligation](../../../docs/releases/tile-count-independent-planetary-cog-reads.md#baseline-failure-mechanisms) is converted to green success here; this assertion may not be deleted.
    }

    #[tokio::test]
    async fn remote_extent_reads_are_tile_count_independent() {
        let fixtures = fixtures();
        let regional_local = LocalFileSystem::new_with_prefix(fixtures.temp_dir.path())
            .expect("regional fixture object store should be rooted");
        let planetary_local = LocalFileSystem::new_with_prefix(fixtures.temp_dir.path())
            .expect("planetary fixture object store should be rooted");
        let regional_store = CogFixtureCountingStore::new(Arc::new(regional_local));
        let planetary_store = CogFixtureCountingStore::new(Arc::new(planetary_local));

        assert!(std::hint::black_box(PLANETARY_TILE_COUNT) / REGIONAL_TILE_COUNT >= 1_000);
        let mut regional_file = File::open(
            fixtures
                .temp_dir
                .path()
                .join(fixtures.regional_object_path.as_ref()),
        )
        .expect("regional fixture should be readable");
        regional_file
            .seek(SeekFrom::Start(200))
            .expect("regional IFD count should be seekable");
        let mut regional_count = [0_u8; 8];
        regional_file
            .read_exact(&mut regional_count)
            .expect("regional IFD count should be readable");
        let mut planetary_file = File::open(
            fixtures
                .temp_dir
                .path()
                .join(fixtures.planetary_object_path.as_ref()),
        )
        .expect("planetary fixture should be readable");
        planetary_file
            .seek(SeekFrom::Start(200))
            .expect("planetary IFD count should be seekable");
        let mut planetary_count = [0_u8; 8];
        planetary_file
            .read_exact(&mut planetary_count)
            .expect("planetary IFD count should be readable");
        assert_eq!(u64::from_le_bytes(regional_count), 19);
        assert_eq!(u64::from_le_bytes(planetary_count), 19);

        let regional_extent = read_remote_extent(
            &regional_store as &dyn ObjectStore,
            &fixtures.regional_object_path,
        )
        .await
        .expect("regional extent should parse");
        let planetary_extent = read_remote_extent(
            &planetary_store as &dyn ObjectStore,
            &fixtures.planetary_object_path,
        )
        .await
        .expect("planetary extent should parse");

        assert_eq!(
            regional_extent.rect(),
            Rect::new(
                coord! { x: 0.0, y: -16_384.0 },
                coord! { x: 16_384.0, y: 0.0 }
            )
        );
        assert_eq!(
            planetary_extent.rect(),
            Rect::new(
                coord! { x: 0.0, y: -500_000.0 },
                coord! { x: 1_070_000.0, y: 0.0 }
            )
        );
        assert_eq!(
            regional_store.requested_range_bytes(),
            planetary_store.requested_range_bytes()
        );
        assert_eq!(
            regional_store.consumed_range_bytes(),
            planetary_store.consumed_range_bytes()
        );
        assert_eq!(regional_store.requested_range_bytes(), 476);
        assert_eq!(regional_store.consumed_range_bytes(), 476);
        assert_eq!(planetary_store.requested_range_bytes(), 476);
        assert_eq!(planetary_store.consumed_range_bytes(), 476);
        assert!(regional_store.requested_range_bytes() < 3_998);
        assert_eq!(
            (
                regional_store.head_calls(),
                regional_store.get_range_calls(),
                regional_store.get_ranges_calls()
            ),
            (1, 3, 1)
        );
        assert_eq!(
            (
                planetary_store.head_calls(),
                planetary_store.get_range_calls(),
                planetary_store.get_ranges_calls()
            ),
            (1, 3, 1)
        );
    }

    #[tokio::test]
    async fn classic_remote_extent_reads_owned_layout() {
        let fixtures = fixtures();
        let local_store = LocalFileSystem::new_with_prefix(fixtures.temp_dir.path())
            .expect("fixture object store should be rooted");
        let store = CogFixtureCountingStore::new(Arc::new(local_store));
        let path = ObjectPath::from("classic.tif");

        let extent = read_remote_extent(&store as &dyn ObjectStore, &path)
            .await
            .expect("classic remote extent should parse");

        assert_eq!(
            extent.rect(),
            Rect::new(coord! { x: 0.0, y: -512.0 }, coord! { x: 512.0, y: 0.0 })
        );
        assert_eq!(
            (
                store.head_calls(),
                store.get_range_calls(),
                store.get_ranges_calls()
            ),
            (1, 3, 1)
        );
        assert_eq!(store.requested_range_bytes(), 282);
        assert_eq!(store.consumed_range_bytes(), 282);
    }

    #[tokio::test]
    async fn planetary_window_resolves_covered_tile_indexes_with_bounded_reads() {
        let fixtures = fixtures();
        let oracle = CrossTileFixtureOracle::new();
        let local_store = LocalFileSystem::new_with_prefix(fixtures.temp_dir.path())
            .expect("fixture object store should be rooted");
        let inner: Arc<dyn ObjectStore> = Arc::new(local_store);
        let store = CogFixtureCountingStore::new(inner);
        let request =
            RasterWindowRequest::new(RasterKind::FlowDir, oracle.one_tile_planetary_bbox());

        let prepared = prepare_window(
            &store as &dyn ObjectStore,
            &fixtures.planetary_object_path,
            &request,
        )
        .await
        .expect("bounded window read should resolve covered index entries");

        // See docs/releases/tile-count-independent-planetary-cog-reads.md for this transition.
        // M3-S4 hardens CogFixtureCountingStore and adds the byte-count backstop proving that
        // only covered index entries are read; these method-call counts do not prove byte volume.
        assert_eq!(prepared.plan.tiles[0].index, 2_039_838);
        assert_eq!(prepared.plan.tiles.len(), 1);
        assert_eq!(prepared.header_bytes, 488);
        assert_eq!(store.requested_range_bytes(), 488);
        assert_eq!(store.consumed_range_bytes(), 488);
        assert_eq!(store.charged_non_range_object_range_bytes(), 0);
        assert_eq!(store.head_calls(), 1);
        assert_eq!(store.get_range_calls(), 3);
        assert_eq!(store.get_ranges_calls(), 2);
    }

    #[tokio::test]
    async fn remote_window_metadata_cost_is_tile_count_independent() {
        let fixtures = fixtures();
        let oracle = CrossTileFixtureOracle::new();
        let regional_bbox = Rect::new(coord! { x: 0.0, y: -1.0 }, coord! { x: 1.0, y: 0.0 });
        let planetary_bbox = oracle.one_tile_planetary_bbox();
        let regional_request = RasterWindowRequest::new(RasterKind::FlowDir, regional_bbox);
        let planetary_request = RasterWindowRequest::new(RasterKind::FlowDir, planetary_bbox);

        let regional_local = LocalFileSystem::new_with_prefix(fixtures.temp_dir.path())
            .expect("regional fixture object store should be rooted");
        let regional_observation_store = CogFixtureCountingStore::new(Arc::new(regional_local));
        let regional_prepared = prepare_window(
            &regional_observation_store,
            &fixtures.regional_object_path,
            &regional_request,
        )
        .await
        .expect("regional covered entries should resolve");
        let planetary_local = LocalFileSystem::new_with_prefix(fixtures.temp_dir.path())
            .expect("planetary fixture object store should be rooted");
        let planetary_observation_store = CogFixtureCountingStore::new(Arc::new(planetary_local));
        let planetary_prepared = prepare_window(
            &planetary_observation_store,
            &fixtures.planetary_object_path,
            &planetary_request,
        )
        .await
        .expect("planetary covered entries should resolve");

        assert_eq!(
            regional_prepared
                .plan
                .tiles
                .iter()
                .map(|tile| tile.index)
                .collect::<Vec<_>>(),
            [0]
        );
        assert_eq!(
            planetary_prepared
                .plan
                .tiles
                .iter()
                .map(|tile| tile.index)
                .collect::<Vec<_>>(),
            [2_039_838]
        );
        let CogIndex::Remote {
            tile_offsets: regional_offsets,
            tile_byte_counts: regional_byte_counts,
        } = regional_prepared.metadata.index
        else {
            panic!("regional fixture should use remote index descriptors");
        };
        let CogIndex::Remote {
            tile_offsets: planetary_offsets,
            tile_byte_counts: planetary_byte_counts,
        } = planetary_prepared.metadata.index
        else {
            panic!("planetary fixture should use remote index descriptors");
        };
        assert_eq!(regional_offsets.count, 1_024);
        assert_eq!(regional_byte_counts.count, 1_024);
        assert_eq!(planetary_offsets.count, 2_041_930);
        assert_eq!(planetary_byte_counts.count, 2_041_930);
        assert_eq!(planetary_offsets.count / regional_offsets.count, 1_994);
        assert!(planetary_offsets.count / regional_offsets.count >= 1_000);
        assert_eq!(regional_offsets.element_width, 8);
        assert_eq!(regional_byte_counts.element_width, 4);
        assert_eq!(planetary_offsets.element_width, 8);
        assert_eq!(planetary_byte_counts.element_width, 4);
        assert_eq!(
            (
                regional_observation_store.head_calls(),
                regional_observation_store.non_range_get_calls(),
                regional_observation_store.get_range_calls(),
                regional_observation_store.get_ranges_calls(),
            ),
            (1, 0, 3, 2)
        );
        assert_eq!(
            (
                planetary_observation_store.head_calls(),
                planetary_observation_store.non_range_get_calls(),
                planetary_observation_store.get_range_calls(),
                planetary_observation_store.get_ranges_calls(),
            ),
            (1, 0, 3, 2)
        );

        let regional_cost =
            measure_window_read_cost(fixtures, &fixtures.regional_object_path, regional_bbox).await;
        assert!(
            regional_cost.total_consumed_bytes < REMOTE_WINDOW_BYTE_BACKSTOP,
            "{} is not below {}",
            regional_cost.total_consumed_bytes,
            REMOTE_WINDOW_BYTE_BACKSTOP
        );
        assert!(regional_cost.total_object_store_api_calls < REMOTE_WINDOW_API_CALL_BACKSTOP);
        let planetary_cost =
            measure_window_read_cost(fixtures, &fixtures.planetary_object_path, planetary_bbox)
                .await;
        assert!(
            planetary_cost.total_consumed_bytes < REMOTE_WINDOW_BYTE_BACKSTOP,
            "{} is not below {}",
            planetary_cost.total_consumed_bytes,
            REMOTE_WINDOW_BYTE_BACKSTOP
        );
        assert!(planetary_cost.total_object_store_api_calls < REMOTE_WINDOW_API_CALL_BACKSTOP);

        let expected_regional = WindowReadCost {
            header_bytes: 488,
            tile_bytes: 284,
            tile_count: 1,
            head_calls: 1,
            non_range_get_calls: 0,
            get_range_calls: 3,
            get_ranges_calls: 3,
            requested_range_bytes: 772,
            consumed_range_bytes: 772,
            charged_non_range_object_range_bytes: 0,
            total_consumed_bytes: 772,
            total_object_store_api_calls: 7,
        };
        assert_eq!(regional_cost, expected_regional);
        assert_eq!(regional_cost.header_bytes, planetary_cost.header_bytes);
        assert_eq!(regional_cost.header_bytes, 488);
        assert_eq!(regional_cost.tile_count, planetary_cost.tile_count);
        assert_eq!(planetary_cost.tile_count, 1);
        assert_eq!(regional_cost.head_calls, planetary_cost.head_calls);
        assert_eq!(
            regional_cost.non_range_get_calls,
            planetary_cost.non_range_get_calls
        );
        assert_eq!(
            regional_cost.get_range_calls,
            planetary_cost.get_range_calls
        );
        assert_eq!(
            regional_cost.get_ranges_calls,
            planetary_cost.get_ranges_calls
        );
        assert_eq!(planetary_cost.tile_bytes, 434);
        assert_eq!(planetary_cost.requested_range_bytes, 922);
        assert_eq!(planetary_cost.consumed_range_bytes, 922);
        assert_eq!(planetary_cost.total_consumed_bytes, 922);
        assert_eq!(planetary_cost.total_object_store_api_calls, 7);
        let regional_fixed_layout_bytes = regional_cost.header_bytes
            - (regional_offsets.element_width + regional_byte_counts.element_width);
        let planetary_fixed_layout_bytes = planetary_cost.header_bytes
            - (planetary_offsets.element_width + planetary_byte_counts.element_width);
        assert_eq!(regional_fixed_layout_bytes, planetary_fixed_layout_bytes);
        assert_eq!(planetary_fixed_layout_bytes, 476);
    }

    #[tokio::test]
    async fn remote_window_cost_scales_only_with_covered_tiles() {
        let fixtures = fixtures();
        let one_tile_bbox = Rect::new(
            coord! { x: 1_069_057.0, y: -499_202.0 },
            coord! { x: 1_069_058.0, y: -499_201.0 },
        );
        let two_tile_bbox = Rect::new(
            coord! { x: 1_069_567.0, y: -499_202.0 },
            coord! { x: 1_069_569.0, y: -499_201.0 },
        );

        let one_tile_local = LocalFileSystem::new_with_prefix(fixtures.temp_dir.path())
            .expect("planetary fixture object store should be rooted");
        let one_tile_observation_store = CogFixtureCountingStore::new(Arc::new(one_tile_local));
        let one_tile_request = RasterWindowRequest::new(RasterKind::FlowDir, one_tile_bbox);
        let one_tile_prepared = prepare_window(
            &one_tile_observation_store,
            &fixtures.planetary_object_path,
            &one_tile_request,
        )
        .await
        .expect("one-tile covered entries should resolve");
        let two_tile_local = LocalFileSystem::new_with_prefix(fixtures.temp_dir.path())
            .expect("planetary fixture object store should be rooted");
        let two_tile_observation_store = CogFixtureCountingStore::new(Arc::new(two_tile_local));
        let two_tile_request = RasterWindowRequest::new(RasterKind::FlowDir, two_tile_bbox);
        let two_tile_prepared = prepare_window(
            &two_tile_observation_store,
            &fixtures.planetary_object_path,
            &two_tile_request,
        )
        .await
        .expect("two-tile covered entries should resolve");

        assert_eq!(
            one_tile_prepared
                .plan
                .tiles
                .iter()
                .map(|tile| tile.index)
                .collect::<Vec<_>>(),
            [2_039_838]
        );
        assert_eq!(
            two_tile_prepared
                .plan
                .tiles
                .iter()
                .map(|tile| tile.index)
                .collect::<Vec<_>>(),
            [2_039_838, 2_039_839]
        );

        let one_tile_cost =
            measure_window_read_cost(fixtures, &fixtures.planetary_object_path, one_tile_bbox)
                .await;
        let two_tile_cost =
            measure_window_read_cost(fixtures, &fixtures.planetary_object_path, two_tile_bbox)
                .await;

        let observed_delta =
            two_tile_cost.total_consumed_bytes - one_tile_cost.total_consumed_bytes;
        assert_eq!(observed_delta, 446);
        let header_delta = two_tile_cost.header_bytes - one_tile_cost.header_bytes;
        let tile_delta = two_tile_cost.tile_bytes - one_tile_cost.tile_bytes;
        assert_eq!(header_delta, 12);
        assert_eq!(tile_delta, 434);
        assert_eq!(observed_delta, header_delta + tile_delta);

        assert_eq!(one_tile_cost.header_bytes, 488);
        assert_eq!(one_tile_cost.tile_bytes, 434);
        assert_eq!(one_tile_cost.tile_count, 1);
        assert_eq!(one_tile_cost.total_consumed_bytes, 922);
        assert_eq!(
            (
                one_tile_cost.head_calls,
                one_tile_cost.non_range_get_calls,
                one_tile_cost.get_range_calls,
                one_tile_cost.get_ranges_calls,
            ),
            (1, 0, 3, 3)
        );
        assert_eq!(one_tile_cost.requested_range_bytes, 922);
        assert_eq!(one_tile_cost.consumed_range_bytes, 922);
        assert_eq!(one_tile_cost.charged_non_range_object_range_bytes, 0);
        assert_eq!(one_tile_cost.total_object_store_api_calls, 7);

        assert_eq!(two_tile_cost.header_bytes, 500);
        assert_eq!(two_tile_cost.tile_bytes, 868);
        assert_eq!(two_tile_cost.tile_count, 2);
        assert_eq!(two_tile_cost.total_consumed_bytes, 1_368);
        assert_eq!(
            (
                two_tile_cost.head_calls,
                two_tile_cost.non_range_get_calls,
                two_tile_cost.get_range_calls,
                two_tile_cost.get_ranges_calls,
            ),
            (1, 0, 3, 3)
        );
        assert_eq!(two_tile_cost.requested_range_bytes, 1_368);
        assert_eq!(two_tile_cost.consumed_range_bytes, 1_368);
        assert_eq!(two_tile_cost.charged_non_range_object_range_bytes, 0);
        assert_eq!(two_tile_cost.total_object_store_api_calls, 7);

        assert!(one_tile_cost.total_consumed_bytes < REMOTE_WINDOW_BYTE_BACKSTOP);
        assert!(one_tile_cost.total_object_store_api_calls < REMOTE_WINDOW_API_CALL_BACKSTOP);
        assert!(two_tile_cost.total_consumed_bytes < REMOTE_WINDOW_BYTE_BACKSTOP);
        assert!(two_tile_cost.total_object_store_api_calls < REMOTE_WINDOW_API_CALL_BACKSTOP);
    }

    #[tokio::test]
    async fn planetary_cache_window_materializes_with_bounded_reads() {
        let fixtures = fixtures();
        let oracle = CrossTileFixtureOracle::new();
        let local_store = LocalFileSystem::new_with_prefix(fixtures.temp_dir.path())
            .expect("fixture object store should be rooted");
        let inner: Arc<dyn ObjectStore> = Arc::new(local_store);
        let store = CogFixtureCountingStore::new(inner);
        let cache_temp = tempfile::TempDir::new().expect("cache temp directory should be created");
        let cache = crate::raster_cache::RemoteRasterCache::new(cache_temp.path().to_path_buf());
        let request =
            RasterWindowRequest::new(RasterKind::FlowDir, oracle.one_tile_planetary_bbox());

        let localized = cache
            .get_or_fetch_window(
                &store,
                &fixtures.planetary_object_path,
                &request,
                "test-fabric",
                "0.1.0",
            )
            .await
            .expect("cache route should materialize a bounded window");

        assert!(localized.path().exists());
        assert!(localized.header_bytes() > 0);
        assert!(localized.tile_bytes() > 0);
        let mut decoder = Decoder::new(File::open(localized.path()).unwrap()).unwrap();
        assert!(decoder.dimensions().unwrap().0 > 0);
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

        let plan = TilePlan::for_window(&meta, window, &ObjectPath::from("geometry.tif")).unwrap();

        assert_eq!(plan.indices, vec![0, 1, 4, 5]);
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
