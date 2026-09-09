//! clean_topology : MultiPolygon × CleanEpsilon → MultiPolygon, morphological closing.
//!
//! Offset outward then inward with mitre joins. The offset engine normalizes
//! shell/hole winding on each pass and returns every noncollapsed polygon part.
//! Epsilon is unchanged; this operation adds no round joins or area threshold.

use geo::algorithm::buffer::{BufferStyle, LineJoin};
use geo::{BooleanOps, Buffer, MultiPolygon};
use tracing::{debug, instrument};

use crate::algo::clean_epsilon::CleanEpsilon;

/// Minimum corner angle for the existing mitre limit of five: 2 asin(1 / 5).
/// This is equivalent to the former denominator threshold 1 + cos(angle) = 0.08.
const MIN_MITRE_ANGLE: f64 = 0.402_715_841_580_661_6;

/// Close micro-gaps by an outward offset followed by an equal inward offset.
///
/// Buffering operates on the full region. In particular, an eroded narrow neck
/// can split into multiple polygons; all resulting regions are retained. Each
/// offset uses OGC shell/hole reconstruction, not a largest-fragment selection.
#[instrument(skip(geom))]
pub fn clean_topology(geom: MultiPolygon<f64>, epsilon: CleanEpsilon) -> MultiPolygon<f64> {
    let distance = epsilon.as_f64();
    debug!(
        epsilon = distance,
        polygon_count = geom.0.len(),
        "cleaning topology"
    );
    let expanded = buffer_multi_polygon(&geom, distance);
    let result = buffer_multi_polygon(&expanded, -distance);
    debug!(polygon_count = result.0.len(), "topology cleaned");
    result
}

fn buffer_multi_polygon(geometry: &MultiPolygon<f64>, distance: f64) -> MultiPolygon<f64> {
    let offset = geometry
        .buffer_with_style(BufferStyle::new(distance).line_join(LineJoin::Miter(MIN_MITRE_ANGLE)));
    // The offset mesh can contain endpoint pinches. Reconstruct OGC rings before
    // another offset or the hole policy consumes it. Buffer output is one region,
    // so even-odd reconstruction does not flatten independent overlapping parts.
    offset.union(&MultiPolygon::new(vec![]))
}

#[cfg(test)]
mod tests {
    use super::{buffer_multi_polygon, clean_topology};
    use crate::algo::clean_epsilon::{CleanEpsilon, DEFAULT_CLEANING_EPSILON};
    use geo::{Area, LineString, MultiPolygon, Polygon};

    fn unit_square() -> Polygon<f64> {
        Polygon::new(
            LineString::from(vec![
                (0.0, 0.0),
                (1.0, 0.0),
                (1.0, 1.0),
                (0.0, 1.0),
                (0.0, 0.0),
            ]),
            vec![],
        )
    }

    #[test]
    fn cleanup_square_is_independent_of_input_winding() {
        use geo::algorithm::winding_order::Winding;
        let ccw = unit_square();
        let mut cw = ccw.clone();
        cw.exterior_mut(|ring| ring.make_cw_winding());
        for polygon in [ccw, cw] {
            let result = clean_topology(MultiPolygon::new(vec![polygon]), CleanEpsilon::new(0.1));
            assert!(
                (result.unsigned_area() - 1.0).abs() < 1e-8,
                "buffer-unbuffer changed square area to {}",
                result.unsigned_area()
            );
        }
    }

    #[test]
    fn erosion_retains_both_lobes_when_a_neck_splits() {
        let polygon = Polygon::new(
            LineString::from(vec![
                (0.0, 0.0),
                (1.0, 0.0),
                (1.0, 0.45),
                (2.0, 0.45),
                (2.0, 0.0),
                (3.0, 0.0),
                (3.0, 1.0),
                (2.0, 1.0),
                (2.0, 0.55),
                (1.0, 0.55),
                (1.0, 1.0),
                (0.0, 1.0),
                (0.0, 0.0),
            ]),
            vec![],
        );
        let result = buffer_multi_polygon(&MultiPolygon::new(vec![polygon]), -0.1);
        assert_eq!(
            result.0.len(),
            2,
            "both noncollapsed lobes must survive erosion"
        );
    }

    #[test]
    fn clean_topology_preserves_shape() {
        let poly = unit_square();
        let mp = MultiPolygon::new(vec![poly.clone()]);
        let original_area = poly.unsigned_area();

        let cleaned = clean_topology(mp, DEFAULT_CLEANING_EPSILON);

        assert!(!cleaned.0.is_empty(), "result should not be empty");
        let cleaned_area: f64 = cleaned.0.iter().map(|p| p.unsigned_area()).sum();

        // Area should be within 1% of original after buffer-unbuffer with tiny epsilon
        let ratio = (cleaned_area - original_area).abs() / original_area;
        assert!(
            ratio < 0.01,
            "area changed by {:.2}%, expected < 1%",
            ratio * 100.0
        );
    }

    #[test]
    fn closing_preserves_a_satellite_smaller_than_epsilon() {
        let satellite = Polygon::new(
            LineString::from(vec![
                (3.0, 0.0),
                (3.000001, 0.0),
                (3.000001, 0.000001),
                (3.0, 0.000001),
                (3.0, 0.0),
            ]),
            vec![],
        );
        let result = clean_topology(
            MultiPolygon::new(vec![unit_square(), satellite]),
            DEFAULT_CLEANING_EPSILON,
        );
        assert_eq!(result.0.len(), 2);
        let smallest = result
            .0
            .iter()
            .map(Area::unsigned_area)
            .fold(f64::INFINITY, f64::min);
        assert!(smallest > 0.9e-12, "satellite lost area: {smallest}");
    }

    #[test]
    fn clean_topology_empty() {
        let mp: MultiPolygon<f64> = MultiPolygon::new(vec![]);
        let result = clean_topology(mp, DEFAULT_CLEANING_EPSILON);
        assert!(result.0.is_empty(), "empty input should yield empty output");
    }

    #[test]
    fn clean_topology_closes_sliver() {
        // Two rectangles with a tiny gap (0.0001 degrees wide)
        let rect1 = Polygon::new(
            LineString::from(vec![
                (0.0, 0.0),
                (1.0, 0.0),
                (1.0, 1.0),
                (0.0, 1.0),
                (0.0, 0.0),
            ]),
            vec![],
        );
        let rect2 = Polygon::new(
            LineString::from(vec![
                (1.0001, 0.0),
                (2.0, 0.0),
                (2.0, 1.0),
                (1.0001, 1.0),
                (1.0001, 0.0),
            ]),
            vec![],
        );

        let mp = MultiPolygon::new(vec![rect1, rect2]);
        assert_eq!(mp.0.len(), 2, "input should have 2 polygons");

        // Use epsilon larger than the gap (0.001 > 0.0001)
        let cleaned = clean_topology(mp, CleanEpsilon::new(0.001));

        // The gap should be closed — result should be a single merged polygon
        assert_eq!(
            cleaned.0.len(),
            1,
            "gap should be closed, expected 1 polygon but got {}",
            cleaned.0.len()
        );
    }
}
