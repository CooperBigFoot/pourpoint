//! WKB decoding re-exported from `pourpoint-core`.
//!
//! This module re-exports WKB decoding from `pourpoint_core::algo::wkb` for
//! convenience. The implementation is pure Rust with no GDAL dependency.

pub use pourpoint_core::algo::wkb::{
    WkbDecodeError, decode_wkb, decode_wkb_multi_polygon, decode_wkb_polygon,
};
