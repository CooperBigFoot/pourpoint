//! Pour-point snapping to the nearest high-accumulation cell.

use hfx::FlowAccumulationUnits;
use tracing::{debug, info, instrument};

use crate::algo::accumulation_tile::AccumulationTile;
use crate::algo::coord::GridCoord;
use crate::algo::projection::NativeCoord;
use crate::algo::snap_threshold::SnapThreshold;
use crate::algo::tile_state::Masked;

/// Errors from pour-point snapping.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SnapError {
    /// Fired when no flow-accumulation cell within the catchment mask reaches the effective threshold.
    #[error(
        "no cell above effective threshold {threshold} {units} within catchment mask near native EPSG:{epsg} x={outlet_x}, y={outlet_y}"
    )]
    NoCellAboveThreshold {
        /// Effective accumulation threshold in the declared units.
        threshold: f32,
        /// Declared flow-accumulation units.
        units: FlowAccumulationUnits,
        /// Numeric declared EPSG identifier.
        epsg: u32,
        /// Native x coordinate of the input outlet.
        outlet_x: f64,
        /// Native y coordinate of the input outlet.
        outlet_y: f64,
    },
    /// Fired when the native outlet point falls outside the raster tile extent.
    #[error(
        "native EPSG:{epsg} outlet x={outlet_x}, y={outlet_y} is outside tile extent ({rows}x{cols})"
    )]
    OutletOutOfBounds {
        /// Numeric declared EPSG identifier.
        epsg: u32,
        /// Native x coordinate of the input outlet.
        outlet_x: f64,
        /// Native y coordinate of the input outlet.
        outlet_y: f64,
        /// Number of rows in the tile.
        rows: usize,
        /// Number of columns in the tile.
        cols: usize,
    },
}

/// Result of a successful pour-point snap.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SnappedPoint {
    cell: GridCoord,
    coord: NativeCoord,
    accumulation: f32,
}

impl SnappedPoint {
    /// Returns the row index of the snapped cell.
    pub fn row(&self) -> usize {
        self.cell.row
    }

    /// Returns the column index of the snapped cell.
    pub fn col(&self) -> usize {
        self.cell.col
    }

    /// Returns the native x coordinate of the snapped cell center.
    pub fn x(&self) -> f64 {
        self.coord.x()
    }

    /// Returns the native y coordinate of the snapped cell center.
    pub fn y(&self) -> f64 {
        self.coord.y()
    }

    /// Returns the pixel position as [`GridCoord`].
    pub fn pixel(&self) -> GridCoord {
        self.cell
    }

    /// Returns the native coordinates as [`NativeCoord`].
    pub fn coord(&self) -> NativeCoord {
        self.coord
    }

    /// Returns the flow accumulation value at the snapped cell.
    pub fn accumulation(&self) -> f32 {
        self.accumulation
    }
}

/// Snap an outlet to the nearest high-accumulation cell within a masked accumulation tile.
///
/// Converts `outlet` to fractional pixel coordinates, then scans all cells
/// where the accumulation value is not NaN and `>= threshold.as_f32()`. Picks
/// the nearest cell by squared Euclidean distance in pixel space. Ties are
/// broken by higher accumulation.
///
/// The `accumulation` tile must already be masked — cells outside the
/// catchment have been set to NaN by [`AccumulationTile::apply_mask`], so no
/// separate mask parameter is required.
///
/// # Errors
///
/// | Condition | Error |
/// |-----------|-------|
/// | Outlet falls outside raster extent | [`SnapError::OutletOutOfBounds`] |
/// | No masked cell exceeds threshold | [`SnapError::NoCellAboveThreshold`] |
#[instrument(skip(accumulation))]
pub fn snap_pour_point(
    outlet: NativeCoord,
    accumulation: &AccumulationTile<Masked>,
    threshold: SnapThreshold,
    flow_accumulation_units: FlowAccumulationUnits,
    epsg: u32,
) -> Result<SnappedPoint, SnapError> {
    let dims = accumulation.dims();
    let rows = dims.rows;
    let cols = dims.cols;
    let geo = accumulation.geo();

    // Convert outlet to fractional pixel coordinates
    let (frac_row, frac_col) = geo.coord_to_pixel_f64(outlet);

    // Check bounds — fractional coords must be within [0, rows) x [0, cols)
    if frac_row < 0.0 || frac_col < 0.0 || frac_row >= rows as f64 || frac_col >= cols as f64 {
        return Err(SnapError::OutletOutOfBounds {
            epsg,
            outlet_x: outlet.x(),
            outlet_y: outlet.y(),
            rows,
            cols,
        });
    }

    debug!(frac_row, frac_col, "outlet pixel coordinates");

    let threshold_f32 = match flow_accumulation_units {
        FlowAccumulationUnits::Cells => threshold.as_f32(),
        FlowAccumulationUnits::Km2 => {
            let threshold_cells = threshold.pixels();
            let threshold_km2 = threshold_cells as f64
                * (geo.pixel_width() * geo.pixel_height()).abs()
                / 1_000_000.0;
            threshold_km2 as f32
        }
    };
    let mut best: Option<(usize, usize, f64, f32)> = None; // (row, col, dist_sq, acc)

    for r in 0..rows {
        for c in 0..cols {
            // Get accumulation, skip NaN (masked-out cells are already NaN)
            let acc = accumulation.get_raw(GridCoord::new(r, c));
            if acc.is_nan() || acc < threshold_f32 {
                continue;
            }

            // Squared Euclidean distance in pixel space (use pixel centers: r+0.5, c+0.5)
            let dr = (r as f64 + 0.5) - frac_row;
            let dc = (c as f64 + 0.5) - frac_col;
            let dist_sq = dr * dr + dc * dc;

            let is_better = match best {
                None => true,
                Some((_, _, best_dist, best_acc)) => {
                    dist_sq < best_dist || (dist_sq == best_dist && acc > best_acc)
                }
            };

            if is_better {
                best = Some((r, c, dist_sq, acc));
            }
        }
    }

    match best {
        Some((row, col, _, acc)) => {
            let coord = geo.pixel_to_coord(GridCoord::new(row, col));
            info!(
                row,
                col,
                x = coord.x(),
                y = coord.y(),
                accumulation = acc,
                "pour point snapped"
            );
            Ok(SnappedPoint {
                cell: GridCoord::new(row, col),
                coord,
                accumulation: acc,
            })
        }
        None => Err(SnapError::NoCellAboveThreshold {
            threshold: threshold_f32,
            units: flow_accumulation_units,
            epsg,
            outlet_x: outlet.x(),
            outlet_y: outlet.y(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::algo::catchment_mask::CatchmentMask;
    use crate::algo::coord::{GridCoord, GridDims};
    use crate::algo::geo_transform::GeoTransform;
    use crate::algo::projection::NativeCoord;
    use crate::algo::raster_tile::RasterTile;

    fn simple_geo() -> GeoTransform {
        GeoTransform::new(NativeCoord::new(0.0, 0.0), 1.0, -1.0)
    }

    // Test 1: single candidate above threshold is selected
    #[test]
    fn single_candidate() {
        let mut tile = AccumulationTile::new(GridDims::new(3, 3), simple_geo()).unwrap();
        tile.set_raw(GridCoord::new(1, 1), 1000.0);
        let mask = CatchmentMask::new(vec![true; 9], GridDims::new(3, 3));
        let masked = tile.apply_mask(&mask).unwrap();
        let result = snap_pour_point(
            NativeCoord::new(1.5, -1.5),
            &masked,
            SnapThreshold::new(500),
            FlowAccumulationUnits::Cells,
            4326_u32,
        )
        .unwrap();
        assert_eq!(result.pixel(), GridCoord::new(1, 1));
        assert_eq!(result.accumulation(), 1000.0);
    }

    // Test 2: nearest of multiple candidates above threshold is selected
    #[test]
    fn nearest_of_multiple() {
        let data = vec![
            600.0_f32,
            f32::NAN,
            700.0, // row 0: (0,0)=600, (0,1)=NaN, (0,2)=700
            f32::NAN,
            f32::NAN,
            f32::NAN, // row 1: all NaN
            f32::NAN,
            f32::NAN,
            800.0, // row 2: (2,2)=800
        ];
        let raw = RasterTile::from_vec(data, GridDims::new(3, 3), f32::NAN, simple_geo()).unwrap();
        let tile = AccumulationTile::from_raw(raw);
        let mask = CatchmentMask::new(vec![true; 9], GridDims::new(3, 3));
        let masked = tile.apply_mask(&mask).unwrap();
        // Outlet very close to (0,2): outlet_x=2.5, outlet_y=-0.5
        let result = snap_pour_point(
            NativeCoord::new(2.5, -0.5),
            &masked,
            SnapThreshold::new(500),
            FlowAccumulationUnits::Cells,
            4326_u32,
        )
        .unwrap();
        assert_eq!(result.pixel(), GridCoord::new(0, 2));
    }

    // Test 3: tie between equidistant cells is broken by higher accumulation
    #[test]
    fn tie_break_by_accumulation() {
        let data = vec![
            f32::NAN,
            f32::NAN,
            f32::NAN, // row 0
            600.0,
            f32::NAN,
            800.0, // row 1: (1,0)=600, (1,2)=800
            f32::NAN,
            f32::NAN,
            f32::NAN, // row 2
        ];
        let raw = RasterTile::from_vec(data, GridDims::new(3, 3), f32::NAN, simple_geo()).unwrap();
        let tile = AccumulationTile::from_raw(raw);
        let mask = CatchmentMask::new(vec![true; 9], GridDims::new(3, 3));
        let masked = tile.apply_mask(&mask).unwrap();
        // Outlet at center of grid: outlet_x=1.5, outlet_y=-1.5
        // (1,0) center is at (0.5, -1.5), (1,2) center is at (2.5, -1.5)
        // Both are equidistant from outlet (1.5, -1.5) — dist_sq = 1.0 each
        let result = snap_pour_point(
            NativeCoord::new(1.5, -1.5),
            &masked,
            SnapThreshold::new(500),
            FlowAccumulationUnits::Cells,
            4326_u32,
        )
        .unwrap();
        assert_eq!(
            result.pixel(),
            GridCoord::new(1, 2),
            "should prefer higher accumulation on tie"
        );
    }

    // Test 4: mask constrains which cells are eligible
    #[test]
    fn mask_constrains_search() {
        let data = vec![
            1000.0,
            f32::NAN,
            f32::NAN, // row 0: (0,0)=1000
            f32::NAN,
            f32::NAN,
            f32::NAN, // row 1
            f32::NAN,
            f32::NAN,
            900.0, // row 2: (2,2)=900
        ];
        let raw = RasterTile::from_vec(data, GridDims::new(3, 3), f32::NAN, simple_geo()).unwrap();
        let tile = AccumulationTile::from_raw(raw);
        // (0,0) is masked out; only (2,2) is eligible
        let mut mask_data = vec![false; 9];
        mask_data[8] = true; // only (2,2) is eligible
        let mask = CatchmentMask::new(mask_data, GridDims::new(3, 3));
        let masked = tile.apply_mask(&mask).unwrap();
        let result = snap_pour_point(
            NativeCoord::new(0.5, -0.5),
            &masked,
            SnapThreshold::new(500),
            FlowAccumulationUnits::Cells,
            4326_u32,
        )
        .unwrap();
        assert_eq!(result.pixel(), GridCoord::new(2, 2));
    }

    // Test 5: no candidates returns NoCellAboveThreshold error
    #[test]
    fn no_candidates_error() {
        let data = vec![100.0_f32; 9];
        let raw = RasterTile::from_vec(data, GridDims::new(3, 3), f32::NAN, simple_geo()).unwrap();
        let tile = AccumulationTile::from_raw(raw);
        let mask = CatchmentMask::new(vec![true; 9], GridDims::new(3, 3));
        let masked = tile.apply_mask(&mask).unwrap();
        let err = snap_pour_point(
            NativeCoord::new(1.5, -1.5),
            &masked,
            SnapThreshold::new(500),
            FlowAccumulationUnits::Cells,
            4326_u32,
        )
        .unwrap_err();
        assert!(matches!(err, SnapError::NoCellAboveThreshold { .. }));
    }

    // Test 6: outlet already on a stream cell
    #[test]
    fn outlet_already_on_stream() {
        let mut tile = AccumulationTile::new(GridDims::new(3, 3), simple_geo()).unwrap();
        tile.set_raw(GridCoord::new(1, 1), 1000.0);
        let mask = CatchmentMask::new(vec![true; 9], GridDims::new(3, 3));
        let masked = tile.apply_mask(&mask).unwrap();
        let result = snap_pour_point(
            NativeCoord::new(1.5, -1.5),
            &masked,
            SnapThreshold::new(500),
            FlowAccumulationUnits::Cells,
            4326_u32,
        )
        .unwrap();
        assert_eq!(result.pixel(), GridCoord::new(1, 1));
        assert_eq!(result.accumulation(), 1000.0);
    }

    // Test 7: NaN cells are skipped
    #[test]
    fn nan_skipped() {
        let data = vec![
            f32::NAN,
            f32::NAN, // row 0: all NaN
            600.0,
            f32::NAN, // row 1: (1,0)=600, (1,1)=NaN
        ];
        let raw = RasterTile::from_vec(data, GridDims::new(2, 2), f32::NAN, simple_geo()).unwrap();
        let tile = AccumulationTile::from_raw(raw);
        let mask = CatchmentMask::new(vec![true; 4], GridDims::new(2, 2));
        let masked = tile.apply_mask(&mask).unwrap();
        // Outlet near center: outlet_x=1.0, outlet_y=-1.0
        let result = snap_pour_point(
            NativeCoord::new(1.0, -1.0),
            &masked,
            SnapThreshold::new(500),
            FlowAccumulationUnits::Cells,
            4326_u32,
        )
        .unwrap();
        assert_eq!(result.pixel(), GridCoord::new(1, 0));
    }

    // Test 8: exact threshold boundary — value exactly equal to threshold is accepted
    #[test]
    fn exact_threshold() {
        let data = vec![499.0_f32, 500.0, 501.0];
        let raw = RasterTile::from_vec(data, GridDims::new(1, 3), f32::NAN, simple_geo()).unwrap();
        let tile = AccumulationTile::from_raw(raw);
        let mask = CatchmentMask::new(vec![true; 3], GridDims::new(1, 3));
        let masked = tile.apply_mask(&mask).unwrap();
        // Outlet at center of (0,1): outlet_x=1.5, outlet_y=-0.5
        let result = snap_pour_point(
            NativeCoord::new(1.5, -0.5),
            &masked,
            SnapThreshold::new(500),
            FlowAccumulationUnits::Cells,
            4326_u32,
        )
        .unwrap();
        assert_eq!(
            result.pixel(),
            GridCoord::new(0, 1),
            "cell at threshold should be accepted (>=)"
        );
    }

    // Test 9: outlet outside raster bounds returns OutletOutOfBounds error
    #[test]
    fn outlet_out_of_bounds() {
        let mut tile = AccumulationTile::new(GridDims::new(3, 3), simple_geo()).unwrap();
        tile.set_raw(GridCoord::new(1, 1), 1000.0);
        let mask = CatchmentMask::new(vec![true; 9], GridDims::new(3, 3));
        let masked = tile.apply_mask(&mask).unwrap();
        // outlet_x=10.0, outlet_y=10.0 → frac_row = -10.0 (negative = OOB)
        let err = snap_pour_point(
            NativeCoord::new(10.0, 10.0),
            &masked,
            SnapThreshold::new(500),
            FlowAccumulationUnits::Cells,
            8857_u32,
        )
        .unwrap_err();
        assert_eq!(
            err,
            SnapError::OutletOutOfBounds {
                epsg: 8857,
                outlet_x: 10.0,
                outlet_y: 10.0,
                rows: 3,
                cols: 3,
            }
        );
        assert_eq!(
            err.to_string(),
            "native EPSG:8857 outlet x=10, y=10 is outside tile extent (3x3)"
        );
    }

    // Test 10: all mask entries false → NoCellAboveThreshold even if values are high
    #[test]
    fn empty_mask_error() {
        let data = vec![1000.0_f32; 4];
        let raw = RasterTile::from_vec(data, GridDims::new(2, 2), f32::NAN, simple_geo()).unwrap();
        let tile = AccumulationTile::from_raw(raw);
        let mask = CatchmentMask::new(vec![false; 4], GridDims::new(2, 2));
        let masked = tile.apply_mask(&mask).unwrap();
        let err = snap_pour_point(
            NativeCoord::new(1.0, -1.0),
            &masked,
            SnapThreshold::new(500),
            FlowAccumulationUnits::Cells,
            4326_u32,
        )
        .unwrap_err();
        assert!(matches!(err, SnapError::NoCellAboveThreshold { .. }));
    }

    #[test]
    fn projected_km2_threshold_accepts_boundary_and_rejects_preceding_f32() {
        let threshold_cells = 1_000_u32;
        let pixel_width = 30.0_f64;
        let pixel_height = -30.0_f64;
        let threshold_km2_f64 =
            threshold_cells as f64 * (pixel_width * pixel_height).abs() / 1_000_000.0;
        let threshold_km2_f32 = threshold_km2_f64 as f32;
        let preceding_f32 = f32::from_bits(threshold_km2_f32.to_bits() - 1);
        let geo = GeoTransform::new(NativeCoord::new(100.0, 200.0), pixel_width, pixel_height);
        let mask = CatchmentMask::new(vec![true], GridDims::new(1, 1));

        let boundary =
            RasterTile::from_vec(vec![threshold_km2_f32], GridDims::new(1, 1), f32::NAN, geo)
                .map(AccumulationTile::from_raw)
                .and_then(|tile| tile.apply_mask(&mask))
                .unwrap();
        let snapped = snap_pour_point(
            NativeCoord::new(115.0, 185.0),
            &boundary,
            SnapThreshold::new(threshold_cells),
            FlowAccumulationUnits::Km2,
            8857_u32,
        )
        .expect("formula-derived f32 boundary should be accepted");
        assert_eq!(snapped.pixel(), GridCoord::new(0, 0));

        let preceding =
            RasterTile::from_vec(vec![preceding_f32], GridDims::new(1, 1), f32::NAN, geo)
                .map(AccumulationTile::from_raw)
                .and_then(|tile| tile.apply_mask(&mask))
                .unwrap();
        let err = snap_pour_point(
            NativeCoord::new(115.0, 185.0),
            &preceding,
            SnapThreshold::new(threshold_cells),
            FlowAccumulationUnits::Km2,
            8857_u32,
        )
        .expect_err("preceding f32 should be below the effective threshold");
        assert_eq!(
            err,
            SnapError::NoCellAboveThreshold {
                threshold: threshold_km2_f32,
                units: FlowAccumulationUnits::Km2,
                epsg: 8857,
                outlet_x: 115.0,
                outlet_y: 185.0,
            }
        );
        assert_eq!(
            err.to_string(),
            format!(
                "no cell above effective threshold {threshold_km2_f32} km2 within catchment mask near native EPSG:8857 x=115, y=185"
            )
        );
    }
}
