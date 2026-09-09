//! validity : MultiPolygon → Result<(), GeometryValidityError> (planar OGC topology).
//!
//! Simple closed rings bound disks. Holes must be covered by their shell and have
//! disjoint interiors. Their ring/contact-point incidence graph must be a forest:
//! a cycle separates the polygon interior. A contact shared by several rings is
//! one graph vertex, not a clique. Distinct polygons may share points, not lines
//! or interior. Winding and consecutive duplicate coordinates do not affect validity.
//!
//! Segment and component envelope R-trees prune intersection candidates. Work is
//! output-sensitive, not an unconditional all-segment-pairs scan; adversarial
//! overlapping envelopes can still produce quadratic candidates. Predicates use
//! geo's robust orientation and DE-9IM machinery on finite f64 coordinates, with
//! no tolerance, snapping, coordinate rounding, or geometry repair.

use std::collections::{BTreeMap, BTreeSet};

use geo::coordinate_position::CoordPos;
use geo::dimensions::Dimensions;
use geo::line_intersection::{LineIntersection, line_intersection};
use geo::{BoundingRect, Line, LineString, MultiPolygon, Polygon, Relate};
use rstar::{AABB, RTree, RTreeObject};
use thiserror::Error;
use tracing::instrument;

/// A violation of planar polygon topology. Ring zero denotes the exterior.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum GeometryValidityError {
    /// A coordinate is NaN or infinite, before any geometric predicate is used.
    #[error("polygon {polygon}, ring {ring}, coordinate {coordinate} is not finite")]
    NonFinite {
        polygon: usize,
        ring: usize,
        coordinate: usize,
    },
    /// A nonempty ring is unclosed or has fewer than three distinct vertices.
    #[error("polygon {polygon}, ring {ring} is not a closed surface boundary")]
    DegenerateRing { polygon: usize, ring: usize },
    /// Nonadjacent segments touch/cross, or segments overlap, in one ring.
    #[error(
        "polygon {polygon}, ring {ring}, segments {first_segment} and {second_segment} intersect"
    )]
    RingIntersection {
        polygon: usize,
        ring: usize,
        first_segment: usize,
        second_segment: usize,
    },
    /// Rings cross or share a nonzero length boundary.
    #[error("polygon {polygon}, rings {first_ring} and {second_ring} cross or overlap")]
    RingConflict {
        polygon: usize,
        first_ring: usize,
        second_ring: usize,
    },
    /// A hole has points outside the closed shell disk.
    #[error("polygon {polygon}, hole ring {ring} is outside its shell")]
    HoleOutsideShell { polygon: usize, ring: usize },
    /// Two hole disks intersect in their interiors, including containment.
    #[error("polygon {polygon}, holes {first_ring} and {second_ring} have overlapping interiors")]
    HoleOverlap {
        polygon: usize,
        first_ring: usize,
        second_ring: usize,
    },
    /// The ring/contact incidence graph has a cycle, disconnecting the interior.
    #[error("polygon {polygon} has disconnected interior")]
    DisconnectedInterior { polygon: usize },
    /// Distinct polygon members share interior or a nonzero length boundary.
    #[error("polygons {first_polygon} and {second_polygon} overlap or share a boundary line")]
    PolygonConflict {
        first_polygon: usize,
        second_polygon: usize,
    },
}

#[derive(Clone)]
struct EnvelopeItem {
    envelope: AABB<[f64; 2]>,
    index: usize,
}
impl RTreeObject for EnvelopeItem {
    type Envelope = AABB<[f64; 2]>;
    fn envelope(&self) -> Self::Envelope {
        self.envelope
    }
}

struct Segment {
    line: Line<f64>,
    ring: usize,
    index: usize,
}

fn envelope(line: Line<f64>) -> AABB<[f64; 2]> {
    AABB::from_corners(
        [line.start.x.min(line.end.x), line.start.y.min(line.end.y)],
        [line.start.x.max(line.end.x), line.start.y.max(line.end.y)],
    )
}

fn component_index(polygons: &[Polygon<f64>]) -> RTree<EnvelopeItem> {
    RTree::bulk_load(
        polygons
            .iter()
            .enumerate()
            .filter_map(|(index, polygon)| {
                polygon.bounding_rect().map(|rect| EnvelopeItem {
                    envelope: AABB::from_corners(
                        [rect.min().x, rect.min().y],
                        [rect.max().x, rect.max().y],
                    ),
                    index,
                })
            })
            .collect(),
    )
}

/// Check all rings, polygon interiors, and relationships between polygon members.
/// Empty polygons and an empty MultiPolygon are valid. Nonempty output is a
/// separate application policy. Coordinates must already be in one planar CRS.
///
/// # Errors
/// Returns the first nonfinite coordinate, malformed ring, or OGC topology violation.
#[instrument(skip(geometry), fields(polygons = geometry.0.len()))]
pub fn validate_multi_polygon(geometry: &MultiPolygon<f64>) -> Result<(), GeometryValidityError> {
    // Validate every coordinate before invoking even an envelope predicate.
    for (polygon, part) in geometry.0.iter().enumerate() {
        for (ring, boundary) in std::iter::once(part.exterior())
            .chain(part.interiors())
            .enumerate()
        {
            for (coordinate, c) in boundary.0.iter().enumerate() {
                if !c.x.is_finite() || !c.y.is_finite() {
                    return Err(GeometryValidityError::NonFinite {
                        polygon,
                        ring,
                        coordinate,
                    });
                }
            }
        }
    }
    for (polygon, part) in geometry.0.iter().enumerate() {
        validate_polygon(part, polygon)?;
    }
    let tree = component_index(&geometry.0);
    for item in &tree {
        for other in tree
            .locate_in_envelope_intersecting(&item.envelope)
            .filter(|other| other.index > item.index)
        {
            let relation = geometry.0[item.index].relate(&geometry.0[other.index]);
            if relation.get(CoordPos::Inside, CoordPos::Inside) != Dimensions::Empty
                || relation.get(CoordPos::OnBoundary, CoordPos::OnBoundary)
                    > Dimensions::ZeroDimensional
            {
                return Err(GeometryValidityError::PolygonConflict {
                    first_polygon: item.index,
                    second_polygon: other.index,
                });
            }
        }
    }
    Ok(())
}

fn validate_polygon(part: &Polygon<f64>, polygon: usize) -> Result<(), GeometryValidityError> {
    if part.exterior().0.is_empty() && part.interiors().iter().all(|ring| ring.0.is_empty()) {
        return Ok(());
    }
    let mut rings = Vec::new();
    let mut segments = Vec::new();
    for (ring, boundary) in std::iter::once(part.exterior())
        .chain(part.interiors())
        .enumerate()
    {
        if ring > 0 && boundary.0.is_empty() {
            rings.push(Polygon::empty());
            continue;
        }
        let mut coords = boundary.0.clone();
        coords.dedup();
        if coords.len() < 4 || coords.first() != coords.last() {
            return Err(GeometryValidityError::DegenerateRing { polygon, ring });
        }
        let normalized = LineString::new(coords);
        for (index, line) in normalized.lines().enumerate() {
            segments.push(Segment { line, ring, index });
        }
        rings.push(Polygon::new(normalized, vec![]));
    }
    let tree = RTree::bulk_load(
        segments
            .iter()
            .enumerate()
            .map(|(index, segment)| EnvelopeItem {
                envelope: envelope(segment.line),
                index,
            })
            .collect(),
    );
    let mut contacts: BTreeMap<(u64, u64), BTreeSet<usize>> = BTreeMap::new();
    for (id, first) in segments.iter().enumerate() {
        for candidate in tree
            .locate_in_envelope_intersecting(&envelope(first.line))
            .filter(|c| c.index > id)
        {
            let second = &segments[candidate.index];
            let Some(hit) = line_intersection(first.line, second.line) else {
                continue;
            };
            if first.ring == second.ring {
                let count = rings[first.ring].exterior().0.len() - 1;
                let adjacent = second.index == first.index + 1
                    || (first.index == 0 && second.index == count - 1);
                if adjacent
                    && matches!(
                        hit,
                        LineIntersection::SinglePoint {
                            is_proper: false,
                            ..
                        }
                    )
                {
                    continue;
                }
                return Err(GeometryValidityError::RingIntersection {
                    polygon,
                    ring: first.ring,
                    first_segment: first.index,
                    second_segment: second.index,
                });
            }
            match hit {
                LineIntersection::SinglePoint {
                    intersection,
                    is_proper: false,
                } => {
                    // Canonicalize signed zero without changing any other coordinate.
                    let key = |v: f64| if v == 0.0 { 0 } else { v.to_bits() };
                    let incident = contacts
                        .entry((key(intersection.x), key(intersection.y)))
                        .or_default();
                    incident.insert(first.ring);
                    incident.insert(second.ring);
                }
                _ => {
                    return Err(GeometryValidityError::RingConflict {
                        polygon,
                        first_ring: first.ring,
                        second_ring: second.ring,
                    });
                }
            }
        }
    }
    for ring in 1..rings.len() {
        if !rings[ring].exterior().0.is_empty() && !rings[0].relate(&rings[ring]).is_covers() {
            return Err(GeometryValidityError::HoleOutsideShell { polygon, ring });
        }
    }
    let ring_tree = component_index(&rings);
    for item in ring_tree.iter().filter(|item| item.index > 0) {
        for other in ring_tree
            .locate_in_envelope_intersecting(&item.envelope)
            .filter(|other| other.index > item.index)
        {
            if rings[item.index]
                .relate(&rings[other.index])
                .get(CoordPos::Inside, CoordPos::Inside)
                != Dimensions::Empty
            {
                return Err(GeometryValidityError::HoleOverlap {
                    polygon,
                    first_ring: item.index,
                    second_ring: other.index,
                });
            }
        }
    }
    // Union each contact's incident rings once. Reconnecting an existing component
    // at a distinct contact closes a cycle in the bipartite incidence graph.
    let mut parents: Vec<usize> = (0..rings.len()).collect();
    for incident in contacts.values() {
        let mut iter = incident.iter();
        if let Some(&first) = iter.next() {
            for &second in iter {
                let a = root(&mut parents, first);
                let b = root(&mut parents, second);
                if a == b {
                    return Err(GeometryValidityError::DisconnectedInterior { polygon });
                }
                parents[b] = a;
            }
        }
    }
    Ok(())
}

fn root(parents: &mut [usize], mut node: usize) -> usize {
    while parents[node] != node {
        parents[node] = parents[parents[node]];
        node = parents[node];
    }
    node
}

#[cfg(test)]
mod tests {
    // The contact, connectivity, and polygon-pair fixtures were independently
    // checked against GEOS 3.13.1. In particular, multiple rings at a single
    // contact must not be mistaken for a disconnected-interior cycle.
    use crate::algo::geometry_validity::{GeometryValidityError, validate_multi_polygon};
    use geo::{LineString, MultiPolygon, Polygon};

    fn ring(points: &[(f64, f64)]) -> LineString<f64> {
        LineString::from(points.to_vec())
    }
    fn polygon(points: &[(f64, f64)]) -> Polygon<f64> {
        Polygon::new(ring(points), vec![])
    }
    fn square() -> Polygon<f64> {
        polygon(&[(0., 0.), (10., 0.), (10., 10.), (0., 10.), (0., 0.)])
    }
    fn with_holes(holes: Vec<LineString<f64>>) -> MultiPolygon<f64> {
        MultiPolygon(vec![Polygon::new(square().exterior().clone(), holes)])
    }

    #[test]
    fn empty_and_duplicate_coordinates_are_valid() {
        assert!(validate_multi_polygon(&MultiPolygon(vec![])).is_ok());
        assert!(validate_multi_polygon(&MultiPolygon(vec![Polygon::empty()])).is_ok());
        assert!(
            validate_multi_polygon(&MultiPolygon(vec![polygon(&[
                (0., 0.),
                (10., 0.),
                (10., 0.),
                (10., 10.),
                (0., 10.),
                (0., 0.)
            ])]))
            .is_ok()
        );
    }

    #[test]
    fn empty_holes_are_valid_but_nonempty_holes_need_a_shell() {
        assert!(validate_multi_polygon(&with_holes(vec![LineString::new(vec![])])).is_ok());
        let no_shell = MultiPolygon(vec![Polygon::new(
            LineString::new(vec![]),
            vec![square().exterior().clone()],
        )]);
        assert!(validate_multi_polygon(&no_shell).is_err());
    }

    #[test]
    fn rejects_nonfinite_before_predicates() {
        let value = MultiPolygon(vec![polygon(&[
            (0., 0.),
            (f64::NAN, 1.),
            (2., 0.),
            (0., 0.),
        ])]);
        assert!(matches!(
            validate_multi_polygon(&value),
            Err(GeometryValidityError::NonFinite { .. })
        ));
    }

    #[test]
    fn rejects_crossings_endpoint_touches_and_overlapping_ring_edges() {
        for points in [
            vec![(0., 0.), (4., 4.), (0., 4.), (4., 0.), (0., 0.)],
            vec![
                (0., 0.),
                (4., 0.),
                (2., 2.),
                (4., 4.),
                (0., 4.),
                (2., 2.),
                (0., 0.),
            ],
            vec![(0., 0.), (4., 0.), (2., 0.), (2., 4.), (0., 4.), (0., 0.)],
            vec![(0., 0.), (2., 0.), (0., 0.)],
        ] {
            assert!(validate_multi_polygon(&MultiPolygon(vec![polygon(&points)])).is_err());
        }
    }

    #[test]
    fn shell_hole_single_and_separate_multiple_tangencies_are_valid() {
        let first = ring(&[(0., 5.), (2., 3.), (4., 5.), (2., 7.), (0., 5.)]);
        let second = ring(&[(10., 5.), (8., 3.), (6., 5.), (8., 7.), (10., 5.)]);
        assert!(validate_multi_polygon(&with_holes(vec![first.clone()])).is_ok());
        assert!(validate_multi_polygon(&with_holes(vec![first, second])).is_ok());
    }

    #[test]
    fn three_rings_at_one_contact_do_not_form_a_cycle() {
        let first = ring(&[(0., 5.), (3., 6.), (2., 8.), (0., 5.)]);
        let second = ring(&[(0., 5.), (2., 2.), (3., 4.), (0., 5.)]);
        assert!(validate_multi_polygon(&with_holes(vec![first, second])).is_ok());
    }

    #[test]
    fn interior_holes_sharing_one_point_remain_connected() {
        let holes = vec![
            ring(&[(5., 5.), (7., 5.), (6., 6.), (5., 5.)]),
            ring(&[(5., 5.), (4., 6.), (3., 5.), (5., 5.)]),
            ring(&[(5., 5.), (4., 3.), (6., 3.), (5., 5.)]),
        ];
        assert!(validate_multi_polygon(&with_holes(holes)).is_ok());
    }

    #[test]
    fn hole_contact_tree_does_not_disconnect_interior() {
        let holes = vec![
            ring(&[(1., 5.), (2., 4.), (3., 5.), (2., 6.), (1., 5.)]),
            ring(&[(3., 5.), (4., 4.), (5., 5.), (4., 6.), (3., 5.)]),
            ring(&[(5., 5.), (6., 4.), (7., 5.), (6., 6.), (5., 5.)]),
        ];
        assert!(validate_multi_polygon(&with_holes(holes)).is_ok());
    }

    #[test]
    fn double_touch_disconnects_interior() {
        let hole = ring(&[(0., 5.), (5., 2.), (10., 5.), (5., 8.), (0., 5.)]);
        assert!(matches!(
            validate_multi_polygon(&with_holes(vec![hole])),
            Err(GeometryValidityError::DisconnectedInterior { .. })
        ));
    }

    #[test]
    fn hole_chain_from_shell_to_shell_disconnects_interior() {
        let first = ring(&[(0., 5.), (2., 3.), (5., 5.), (2., 7.), (0., 5.)]);
        let second = ring(&[(5., 5.), (8., 3.), (10., 5.), (8., 7.), (5., 5.)]);
        assert!(matches!(
            validate_multi_polygon(&with_holes(vec![first, second])),
            Err(GeometryValidityError::DisconnectedInterior { .. })
        ));
    }

    #[test]
    fn hole_cycle_disconnects_interior() {
        let a = ring(&[(2., 2.), (5., 2.), (5., 4.), (2., 2.)]);
        let b = ring(&[(5., 2.), (8., 2.), (8., 6.), (5., 2.)]);
        let c = ring(&[(5., 4.), (8., 6.), (2., 8.), (5., 4.)]);
        assert!(matches!(
            validate_multi_polygon(&with_holes(vec![a, b, c])),
            Err(GeometryValidityError::DisconnectedInterior { .. })
        ));
    }

    #[test]
    fn holes_outside_crossing_overlapping_and_nested_are_invalid() {
        let inner = ring(&[(2., 2.), (8., 2.), (8., 8.), (2., 8.), (2., 2.)]);
        for hole in [
            ring(&[(20., 20.), (21., 20.), (21., 21.), (20., 20.)]),
            ring(&[(0., 0.), (5., 0.), (5., 5.), (0., 0.)]),
            ring(&[(-1., 5.), (5., 1.), (11., 5.), (5., 9.), (-1., 5.)]),
        ] {
            assert!(validate_multi_polygon(&with_holes(vec![hole])).is_err());
        }
        for hole in [
            ring(&[(3., 3.), (4., 3.), (4., 4.), (3., 3.)]),
            ring(&[(7., 7.), (9., 7.), (9., 9.), (7., 7.)]),
        ] {
            assert!(validate_multi_polygon(&with_holes(vec![inner.clone(), hole])).is_err());
        }
    }

    #[test]
    fn polygon_pairs_reject_overlap_containment_duplicate_and_shared_line() {
        for other in [
            square(),
            polygon(&[(2., 2.), (3., 2.), (3., 3.), (2., 2.)]),
            polygon(&[(9., 9.), (11., 9.), (11., 11.), (9., 9.)]),
            polygon(&[(10., 0.), (12., 0.), (12., 10.), (10., 10.), (10., 0.)]),
        ] {
            assert!(matches!(
                validate_multi_polygon(&MultiPolygon(vec![square(), other])),
                Err(GeometryValidityError::PolygonConflict { .. })
            ));
        }
    }

    #[test]
    fn three_polygons_at_one_point_and_islands_in_holes_are_valid() {
        let a = polygon(&[(0., 0.), (2., 0.), (1., 1.), (0., 0.)]);
        let b = polygon(&[(0., 0.), (-1., 1.), (-2., 0.), (0., 0.)]);
        let c = polygon(&[(0., 0.), (-1., -2.), (1., -2.), (0., 0.)]);
        assert!(validate_multi_polygon(&MultiPolygon(vec![a, b, c])).is_ok());
        let mut islands = with_holes(vec![ring(&[
            (2., 2.),
            (8., 2.),
            (8., 8.),
            (2., 8.),
            (2., 2.),
        ])]);
        islands
            .0
            .push(polygon(&[(3., 3.), (4., 3.), (4., 4.), (3., 3.)]));
        assert!(validate_multi_polygon(&islands).is_ok());
    }

    #[test]
    fn large_simple_ring_uses_spatial_candidates() {
        let mut coords: Vec<_> = (0..100_000)
            .map(|i| {
                let angle = f64::from(i) * std::f64::consts::TAU / 100_000.;
                (angle.cos(), angle.sin())
            })
            .collect();
        coords.push(coords[0]);
        assert!(validate_multi_polygon(&MultiPolygon(vec![polygon(&coords)])).is_ok());
    }
}
