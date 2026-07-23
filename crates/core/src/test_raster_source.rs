//! Test-only local GeoTIFF raster source for parity fixtures.

use std::path::Path;

use geo::Rect;
use hfx::FlowDirEncoding;

use crate::algo::{
    AccumulationTile, FlowDirectionTile, GridDims, RasterSource, RasterSourceError, RasterTile, Raw,
};
use crate::cog::{LocalWindowData, read_local_geotiff_window};
use crate::error::CacheError;
use crate::session::RasterKind;

/// Local TIFF-backed [`RasterSource`] used only by committed test fixtures.
#[derive(Debug, Clone, Copy, Default)]
pub struct LocalTiffRasterSource;

impl LocalTiffRasterSource {
    /// Return a local TIFF source that decodes flow directions with `encoding`.
    pub fn with_encoding(encoding: FlowDirEncoding) -> EncodedLocalTiffRasterSource {
        EncodedLocalTiffRasterSource { encoding }
    }
}

/// Local TIFF source with an explicitly selected flow-direction encoding.
#[derive(Debug, Clone, Copy)]
pub struct EncodedLocalTiffRasterSource {
    encoding: FlowDirEncoding,
}

impl RasterSource for LocalTiffRasterSource {
    fn load_flow_direction(
        &self,
        uri: &str,
        bbox: &Rect<f64>,
    ) -> Result<FlowDirectionTile<Raw>, RasterSourceError> {
        load_flow_direction(uri, bbox, FlowDirEncoding::Esri)
    }

    fn load_accumulation(
        &self,
        uri: &str,
        bbox: &Rect<f64>,
    ) -> Result<AccumulationTile<Raw>, RasterSourceError> {
        load_accumulation(uri, bbox)
    }
}

impl RasterSource for EncodedLocalTiffRasterSource {
    fn load_flow_direction(
        &self,
        uri: &str,
        bbox: &Rect<f64>,
    ) -> Result<FlowDirectionTile<Raw>, RasterSourceError> {
        load_flow_direction(uri, bbox, self.encoding)
    }

    fn load_accumulation(
        &self,
        uri: &str,
        bbox: &Rect<f64>,
    ) -> Result<AccumulationTile<Raw>, RasterSourceError> {
        load_accumulation(uri, bbox)
    }
}

fn load_flow_direction(
    uri: &str,
    bbox: &Rect<f64>,
    encoding: FlowDirEncoding,
) -> Result<FlowDirectionTile<Raw>, RasterSourceError> {
    let window = read_local_geotiff_window(Path::new(uri), RasterKind::FlowDir, bbox)
        .map_err(|source| map_cache_error(source, uri))?;
    let LocalWindowData::U8(values) = window.data else {
        return Err(RasterSourceError::ReadFailed {
            path: uri.to_string(),
            reason: "flow_dir.tif did not normalize to u8 samples".to_string(),
        });
    };
    let dims = GridDims::new(window.height as usize, window.width as usize);
    let nodata = window.nodata.parse::<u8>().unwrap_or(255);
    let raw = RasterTile::from_vec(values, dims, nodata, window.geo).map_err(|source| {
        RasterSourceError::TileConstruction {
            reason: source.to_string(),
        }
    })?;
    Ok(FlowDirectionTile::from_raw(raw, encoding))
}

fn load_accumulation(
    uri: &str,
    bbox: &Rect<f64>,
) -> Result<AccumulationTile<Raw>, RasterSourceError> {
    let window = read_local_geotiff_window(Path::new(uri), RasterKind::FlowAcc, bbox)
        .map_err(|source| map_cache_error(source, uri))?;
    let LocalWindowData::F32(values) = window.data else {
        return Err(RasterSourceError::ReadFailed {
            path: uri.to_string(),
            reason: "flow_acc.tif did not normalize to f32 samples".to_string(),
        });
    };
    let dims = GridDims::new(window.height as usize, window.width as usize);
    let raw = RasterTile::from_vec(values, dims, f32::NAN, window.geo).map_err(|source| {
        RasterSourceError::TileConstruction {
            reason: source.to_string(),
        }
    })?;
    Ok(AccumulationTile::from_raw(raw))
}

fn map_cache_error(source: CacheError, uri: &str) -> RasterSourceError {
    match source {
        CacheError::Io { source, .. } if source.kind() == std::io::ErrorKind::NotFound => {
            RasterSourceError::FileNotFound {
                path: uri.to_string(),
            }
        }
        CacheError::Io { source, .. } => RasterSourceError::OpenFailed {
            path: uri.to_string(),
            reason: source.to_string(),
        },
        CacheError::Tiff { source, .. } => RasterSourceError::ReadFailed {
            path: uri.to_string(),
            reason: source.to_string(),
        },
        CacheError::UnsupportedCog { reason, .. } => RasterSourceError::ReadFailed {
            path: uri.to_string(),
            reason,
        },
        CacheError::ObjectStore { source, .. } => RasterSourceError::ReadFailed {
            path: uri.to_string(),
            reason: source.to_string(),
        },
        CacheError::Persist { source } => RasterSourceError::ReadFailed {
            path: uri.to_string(),
            reason: source.to_string(),
        },
    }
}
