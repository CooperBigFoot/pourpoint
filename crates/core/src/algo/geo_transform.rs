//! Affine geo-transform for raster grids.

use crate::algo::coord::GridCoord;
use crate::algo::projection::NativeCoord;

/// GDAL-style affine transform (no rotation/shear).
///
/// Stores the top-left corner origin and per-pixel step sizes.
/// `pixel_height` is **negative** for standard north-up rasters
/// because y decreases as row index increases.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeoTransform {
    origin_x: f64,
    origin_y: f64,
    pixel_width: f64,
    pixel_height: f64,
}

impl GeoTransform {
    /// Creates a new geo-transform from a native-coordinate origin and pixel dimensions.
    pub fn new(origin: NativeCoord, pixel_width: f64, pixel_height: f64) -> Self {
        Self {
            origin_x: origin.x(),
            origin_y: origin.y(),
            pixel_width,
            pixel_height,
        }
    }

    /// Returns the area of one pixel in the CRS coordinate units squared.
    pub fn pixel_area(&self) -> f64 {
        (self.pixel_width * self.pixel_height).abs()
    }

    /// Converts a [`GridCoord`] to native coordinates at the pixel center.
    ///
    /// Formula:
    /// ```text
    /// x = origin_x + (col + 0.5) * pixel_width
    /// y = origin_y + (row + 0.5) * pixel_height
    /// ```
    pub fn pixel_to_coord(&self, cell: GridCoord) -> NativeCoord {
        let x = self.origin_x + (cell.col as f64 + 0.5) * self.pixel_width;
        let y = self.origin_y + (cell.row as f64 + 0.5) * self.pixel_height;
        NativeCoord::new(x, y)
    }

    /// Converts a native coordinate to the containing pixel [`GridCoord`].
    ///
    /// Formula:
    /// ```text
    /// col = floor((x - origin_x) / pixel_width)
    /// row = floor((y - origin_y) / pixel_height)
    /// ```
    pub fn coord_to_pixel(&self, coord: NativeCoord) -> GridCoord {
        let col = ((coord.x() - self.origin_x) / self.pixel_width).floor() as usize;
        let row = ((coord.y() - self.origin_y) / self.pixel_height).floor() as usize;
        GridCoord { row, col }
    }

    /// Converts a native coordinate to fractional pixel coordinates `(row, col)`.
    ///
    /// Unlike [`coord_to_pixel`](Self::coord_to_pixel), this does not floor the result,
    /// so the sub-pixel position is preserved.
    pub fn coord_to_pixel_f64(&self, coord: NativeCoord) -> (f64, f64) {
        let col = (coord.x() - self.origin_x) / self.pixel_width;
        let row = (coord.y() - self.origin_y) / self.pixel_height;
        (row, col)
    }

    /// Returns the x coordinate of the top-left corner.
    pub fn origin_x(&self) -> f64 {
        self.origin_x
    }

    /// Returns the y coordinate of the top-left corner.
    pub fn origin_y(&self) -> f64 {
        self.origin_y
    }

    /// Returns the pixel width (cell size in the x direction, always positive).
    pub fn pixel_width(&self) -> f64 {
        self.pixel_width
    }

    /// Returns the pixel height (negative for north-up rasters).
    pub fn pixel_height(&self) -> f64 {
        self.pixel_height
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::algo::coord::GridCoord;
    use crate::algo::projection::NativeCoord;

    const MERIT_RES: f64 = 1.0 / 1200.0;

    fn merit_gt() -> GeoTransform {
        GeoTransform::new(NativeCoord::new(0.0, 0.0), MERIT_RES, -MERIT_RES)
    }

    #[test]
    fn pixel_to_coord_origin_zero() {
        let gt = merit_gt();
        let coord = gt.pixel_to_coord(GridCoord::new(0, 0));
        assert!((coord.x() - 0.5 / 1200.0).abs() < 1e-15);
        assert!((coord.y() - (-0.5 / 1200.0)).abs() < 1e-15);
    }

    #[test]
    fn coord_to_pixel_round_trip() {
        let gt = GeoTransform::new(NativeCoord::new(-180.0, 90.0), MERIT_RES, -MERIT_RES);

        for (row, col) in [(0, 0), (5, 10), (100, 200), (1199, 1199)] {
            let coord = gt.pixel_to_coord(GridCoord::new(row, col));
            let cell = gt.coord_to_pixel(coord);
            assert_eq!(
                (cell.row, cell.col),
                (row, col),
                "round-trip failed for ({row}, {col})"
            );
        }
    }

    #[test]
    fn coord_to_pixel_f64_center() {
        let gt = merit_gt();

        for (row, col) in [(0usize, 0usize), (3, 7), (10, 20)] {
            // Pixel center coordinates.
            let coord = gt.pixel_to_coord(GridCoord::new(row, col));
            let (rf, cf) = gt.coord_to_pixel_f64(coord);
            assert!(
                (rf - (row as f64 + 0.5)).abs() < 1e-10,
                "row fractional mismatch at ({row}, {col}): got {rf}"
            );
            assert!(
                (cf - (col as f64 + 0.5)).abs() < 1e-10,
                "col fractional mismatch at ({row}, {col}): got {cf}"
            );
        }
    }

    #[test]
    fn negative_pixel_height() {
        let gt = GeoTransform::new(NativeCoord::new(0.0, 90.0), 1.0 / 1200.0, -1.0 / 1200.0);
        let coord0 = gt.pixel_to_coord(GridCoord::new(0, 0));
        let coord1 = gt.pixel_to_coord(GridCoord::new(1, 0));
        assert!(
            coord1.y() < coord0.y(),
            "y should decrease as row increases; got y0={}, y1={}",
            coord0.y(),
            coord1.y()
        );
    }

    #[test]
    fn pixel_area_is_positive() {
        let gt = GeoTransform::new(NativeCoord::new(0.0, 0.0), MERIT_RES, -MERIT_RES);
        let area = gt.pixel_area();
        assert!(area > 0.0, "pixel area must be positive, got {area}");
        assert!(
            (area - MERIT_RES * MERIT_RES).abs() < f64::EPSILON,
            "expected {expected}, got {area}",
            expected = MERIT_RES * MERIT_RES
        );
    }
}
