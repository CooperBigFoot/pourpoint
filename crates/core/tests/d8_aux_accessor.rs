use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use geo::{BoundingRect, LineString, MultiPolygon, Polygon, Rect, coord};
use hfx::{EpsgCode, FlowAccumulationUnits, FlowDirEncoding, UnitId};
use pourpoint_core::algo::coord::GeoCoord;
use pourpoint_core::algo::{
    AccumulationTile, Crs, FlowDirectionTile, GeoTransform, GridCoord, GridDims, NativeCoord,
    ProjectionError, RasterSource, RasterSourceError, RasterTile, Raw, RefinementError, SnapError,
    canonical_wkb_multi_polygon, forward, inverse,
};
use pourpoint_core::error::{CacheError, D8NativeCoverageCandidate};
use pourpoint_core::refinement::{
    BestEffortSkipCategory, BestEffortSkipSource, D8RasterRefinementStrategy, D8RefinementPantry,
    TerminalRefinementDecision, TerminalRefinementError, TerminalRefinementInput,
    TerminalRefinementStrategy,
};
use pourpoint_core::session::{DatasetSession, RasterKind};
use pourpoint_core::test_raster_source::LocalTiffRasterSource;
use pourpoint_core::{
    BestEffortSkipReason, DelineationOptions, Engine, EngineError, RefinementMode,
    RefinementOutcome, RefinementProvenance, RefinementStrategyName, SessionError,
};
use serde_json::{Value, json};
use tempfile::TempDir;
use tiff::encoder::{TiffEncoder, colortype};
use tiff::tags::Tag;

const FIXTURE_DIR: &str = "tests/fixtures/parity/v021_synthetic_refined";
// Cross-libm variance is ~1 ulp (~1.8e-15 at magnitude 10). A 1e-12-degree
// tolerance gives >500x headroom while remaining 1000x tighter than the
// engine's 1e-9-degree canonical budget, so real CRS wiring errors still fail
// by many orders of magnitude.
const INVERSE_PROJECTION_TOLERANCE_DEGREES: f64 = 1e-12;

#[test]
fn projected_d8_selection_projects_terminal_before_extent_comparison() {
    let (_tmp, root) = copied_fixture();
    write_projected_manifest(&root);
    write_projected_tiff(&root.join("flow_dir.tif"), FarRasterKind::FlowDir);
    write_projected_tiff(&root.join("flow_acc.tif"), FarRasterKind::FlowAcc);

    let terminal = projected_terminal_with_hole();
    let session = DatasetSession::open_path(&root).expect("temp fixture should open");

    let (handle, native_terminal) = session
        .select_d8_raster_for_terminal(&terminal)
        .expect("projected declaration should cover terminal after projection");

    assert_eq!(handle.declaration_index(), 0);
    let source_interior = &terminal.0[0].interiors()[0];
    let expected_interior = source_interior
        .0
        .iter()
        .map(|coordinate| {
            let native = forward(Crs::Epsg8857, GeoCoord::new(coordinate.x, coordinate.y));
            coord! { x: native.x(), y: native.y() }
        })
        .collect::<Vec<_>>();
    assert_eq!(native_terminal.0[0].interiors()[0].0, expected_interior);
    assert_eq!(
        native_terminal.0[0].interiors()[0].0.first(),
        native_terminal.0[0].interiors()[0].0.last(),
        "projected interior ring must preserve closure"
    );
    assert_eq!(
        native_terminal
            .bounding_rect()
            .expect("native terminal should have bounds"),
        Rect::new(
            coord! { x: 951_078.944_848_778, y: 1_281_580.009_108_443_3 },
            coord! { x: 951_117.540_068_018_6, y: 1_281_631.011_053_289_8 },
        )
    );
}

#[test]
fn declared_d8_accessor_selects_committed_fixture_paths() {
    let root = fixture_path();
    let session = DatasetSession::open_path(&root).expect("fixture should open");
    let bbox = synthetic_full_extent();

    assert!(session.has_d8_aux());
    let (handle, _) = session
        .select_d8_raster_for_terminal(&rect_terminal(bbox))
        .expect("single declared D8 raster should cover fixture bbox");

    assert_eq!(handle.declaration_index(), 0);
    let expected_crs: EpsgCode = "EPSG:4326".parse().unwrap();
    assert_eq!(handle.crs(), &expected_crs);
    assert_eq!(handle.flow_dir_encoding(), FlowDirEncoding::Esri);
    assert_eq!(
        handle.flow_accumulation_units(),
        FlowAccumulationUnits::Cells
    );
    assert!(handle.flow_dir_uri().ends_with("flow_dir.tif"));
    assert!(handle.flow_acc_uri().ends_with("flow_acc.tif"));

    let flow_dir = session
        .localize_d8_raster_window(&handle, RasterKind::FlowDir, bbox)
        .expect("local flow-dir window should resolve to selected declared path");
    let flow_acc = session
        .localize_d8_raster_window(&handle, RasterKind::FlowAcc, bbox)
        .expect("local flow-acc window should resolve to selected declared path");

    assert_eq!(flow_dir.path(), root.join("flow_dir.tif"));
    assert_eq!(flow_acc.path(), root.join("flow_acc.tif"));
}

#[test]
fn multi_decl_selection_skips_non_intersecting_first_decl() {
    let (_tmp, root) = copied_fixture();
    write_far_away_tiff(&root.join("far_flow_dir.tif"), FarRasterKind::FlowDir);
    write_far_away_tiff(&root.join("far_flow_acc.tif"), FarRasterKind::FlowAcc);
    prepend_far_away_d8_decl(&root);

    let session = DatasetSession::open_path(&root).expect("temp fixture should open");
    let (handle, _) = session
        .select_d8_raster_for_terminal(&rect_terminal(synthetic_full_extent()))
        .expect("second declaration should cover bbox");

    assert_eq!(handle.declaration_index(), 1);
    assert!(handle.flow_dir_uri().ends_with("flow_dir.tif"));
    assert!(handle.flow_acc_uri().ends_with("flow_acc.tif"));
}

#[test]
fn inclusive_containment_accepts_bbox_equal_to_raster_extent() {
    let session = DatasetSession::open_path(&fixture_path()).expect("fixture should open");
    let (handle, _) = session
        .select_d8_raster_for_terminal(&rect_terminal(synthetic_full_extent()))
        .expect("bbox equal to raster extent should count as covered");

    assert_eq!(handle.declaration_index(), 0);
}

#[test]
fn multiple_covering_decls_select_manifest_first() {
    // Two declarations fully cover the bbox (the expected case for a per-basin
    // partitioned D8 fabric, where irregular basins have overlapping rectangular
    // extents). hfx.aux.d8_raster.v2 requires overlapping entries to agree in the
    // overlap, so selection collapses to the manifest-first covering declaration
    // rather than erroring.
    let (_tmp, root) = copied_fixture();
    duplicate_committed_d8_decl(&root);
    let session = DatasetSession::open_path(&root).expect("temp fixture should open");

    let (handle, _) = session
        .select_d8_raster_for_terminal(&rect_terminal(synthetic_full_extent()))
        .expect("multiple covering declarations should select manifest-first, not error");

    assert_eq!(handle.declaration_index(), 0);
    assert!(handle.flow_dir_uri().ends_with("flow_dir.tif"));
    assert!(handle.flow_acc_uri().ends_with("flow_acc.tif"));
}

#[test]
fn missing_d8_selection_hard_errors() {
    let (_tmp, root) = copied_fixture();
    remove_d8_aux(&root);
    let session = DatasetSession::open_path(&root).expect("temp fixture without D8 should open");

    let err = session
        .select_d8_raster_for_terminal(&rect_terminal(synthetic_full_extent()))
        .expect_err("explicit D8 selection should require D8 aux");

    assert!(matches!(err, SessionError::MissingRequiredD8Aux));
}

#[test]
fn degenerate_terminal_precedes_missing_d8_declaration() {
    let (_tmp, root) = copied_fixture();
    remove_d8_aux(&root);
    let session = DatasetSession::open_path(&root).expect("temp fixture without D8 should open");
    let terminal = MultiPolygon::new(vec![]);

    let err = D8RasterRefinementStrategy
        .refine_terminal(
            TerminalRefinementInput {
                terminal_unit: UnitId::new(42).expect("valid unit id"),
                terminal_geometry: &terminal,
                resolved_outlet: GeoCoord::new(2.5, -2.5),
                snap_threshold: pourpoint_core::algo::SnapThreshold::DEFAULT,
            },
            &D8RefinementPantry {
                session: &session,
                raster_source: None,
            },
        )
        .expect_err("empty terminal should precede a missing D8 declaration");

    assert!(
        matches!(
            &err,
            TerminalRefinementError::Algorithm {
                unit_id: 42,
                source: RefinementError::DegenerateTerminalPolygon,
            }
        ),
        "expected degenerate terminal before missing D8 declaration; got {err}"
    );
}

#[test]
fn degenerate_terminal_precedes_unsupported_d8_crs() {
    let (_tmp, root) = copied_fixture();
    write_projected_manifest(&root);
    let mut projected = manifest(&root);
    projected["auxiliary"][0]["metadata"]["crs"] = json!("EPSG:3857");
    write_manifest(&root, projected);
    let session = DatasetSession::open_path(&root).expect("temp fixture should open");
    let terminal = MultiPolygon::new(vec![]);

    let err = D8RasterRefinementStrategy
        .refine_terminal(
            TerminalRefinementInput {
                terminal_unit: UnitId::new(42).expect("valid unit id"),
                terminal_geometry: &terminal,
                resolved_outlet: GeoCoord::new(10.0, 10.0),
                snap_threshold: pourpoint_core::algo::SnapThreshold::DEFAULT,
            },
            &D8RefinementPantry {
                session: &session,
                raster_source: None,
            },
        )
        .expect_err("empty terminal should precede unsupported D8 CRS selection");

    assert!(
        matches!(
            &err,
            TerminalRefinementError::Algorithm {
                unit_id: 42,
                source: RefinementError::DegenerateTerminalPolygon,
            }
        ),
        "expected degenerate terminal before EPSG:3857 D8 selection; got {err}"
    );
}

#[test]
fn degenerate_terminal_precedes_out_of_range_d8_crs() {
    let (_tmp, root) = copied_fixture();
    write_projected_manifest(&root);
    let mut projected = manifest(&root);
    projected["auxiliary"][0]["metadata"]["crs"] = json!("EPSG:99999999999");
    write_manifest(&root, projected);
    let session = DatasetSession::open_path(&root).expect("temp fixture should open");
    let terminal = MultiPolygon::new(vec![]);

    let err = D8RasterRefinementStrategy
        .refine_terminal(
            TerminalRefinementInput {
                terminal_unit: UnitId::new(42).expect("valid unit id"),
                terminal_geometry: &terminal,
                resolved_outlet: GeoCoord::new(10.0, 10.0),
                snap_threshold: pourpoint_core::algo::SnapThreshold::DEFAULT,
            },
            &D8RefinementPantry {
                session: &session,
                raster_source: None,
            },
        )
        .expect_err("empty terminal should precede out-of-range D8 CRS selection");

    assert!(
        matches!(
            &err,
            TerminalRefinementError::Algorithm {
                unit_id: 42,
                source: RefinementError::DegenerateTerminalPolygon,
            }
        ),
        "expected degenerate terminal before EPSG:99999999999 D8 selection; got {err}"
    );
}

#[test]
fn unsupported_projected_crs_routes_through_d8_selection() {
    let (_tmp, root) = copied_fixture();
    write_projected_manifest(&root);
    let mut projected = manifest(&root);
    projected["auxiliary"][0]["metadata"]["crs"] = json!("EPSG:3857");
    write_manifest(&root, projected);
    let session = DatasetSession::open_path(&root).expect("temp fixture should open");
    let terminal = projected_terminal();
    let strategy = D8RasterRefinementStrategy;
    let err = strategy
        .refine_terminal(
            TerminalRefinementInput {
                terminal_unit: UnitId::new(42).expect("valid unit id"),
                terminal_geometry: &terminal,
                resolved_outlet: GeoCoord::new(10.0, 10.0),
                snap_threshold: pourpoint_core::algo::SnapThreshold::new(1),
            },
            &D8RefinementPantry {
                session: &session,
                raster_source: None,
            },
        )
        .expect_err("unsupported CRS should fail during selection");
    assert!(matches!(
        err,
        TerminalRefinementError::D8Selection {
            source: SessionError::UnsupportedD8Crs {
                source: ProjectionError::UnsupportedCrs { epsg: 3857 },
                ..
            },
            ..
        }
    ));
    let engine_error = EngineError::from(err);
    assert!(matches!(engine_error, EngineError::D8Selection { .. }));
    assert!(engine_error.to_string().contains("EPSG:3857"));
}

#[test]
fn geographic_km2_routes_through_refinement() {
    let (_tmp, root) = copied_fixture();
    let mut fixture_manifest = manifest(&root);
    fixture_manifest["auxiliary"][0]["metadata"]["crs"] = json!("EPSG:4326");
    fixture_manifest["auxiliary"][0]["metadata"]["flow_acc_units"] = json!("km2");
    write_manifest(&root, fixture_manifest);
    let session = DatasetSession::open_path(&root).expect("temp fixture should open");
    let terminal = rect_terminal(synthetic_full_extent());
    let source = LocalTiffRasterSource;
    let err = D8RasterRefinementStrategy
        .refine_terminal(
            TerminalRefinementInput {
                terminal_unit: UnitId::new(42).expect("valid unit id"),
                terminal_geometry: &terminal,
                resolved_outlet: GeoCoord::new(2.5, -2.5),
                snap_threshold: pourpoint_core::algo::SnapThreshold::new(1),
            },
            &D8RefinementPantry {
                session: &session,
                raster_source: Some(&source),
            },
        )
        .expect_err("geographic km2 should fail as a refinement error");
    assert!(matches!(
        err,
        TerminalRefinementError::Algorithm {
            source: pourpoint_core::algo::RefinementError::GeographicKm2Unsupported {
                epsg: 4326,
                units: FlowAccumulationUnits::Km2,
            },
            ..
        }
    ));
    let engine_error = EngineError::from(err);
    assert!(matches!(engine_error, EngineError::Refinement { .. }));
    assert!(engine_error.to_string().contains("EPSG:4326"));
    assert!(engine_error.to_string().contains("km2"));
}

#[test]
fn projected_refinement_carves_natively_and_returns_geographic_output() {
    let (_tmp, root) = copied_fixture();
    write_projected_manifest(&root);
    let mut projected = manifest(&root);
    projected["auxiliary"][0]["metadata"]["flow_acc_units"] = json!("cells");
    write_manifest(&root, projected);
    write_projected_tiff(&root.join("flow_dir.tif"), FarRasterKind::FlowDir);
    write_projected_tiff(&root.join("flow_acc.tif"), FarRasterKind::FlowAcc);
    let session = DatasetSession::open_path(&root).expect("temp fixture should open");
    let terminal = projected_terminal();
    let source = ProjectedRasterSource::default();

    let decision = D8RasterRefinementStrategy
        .refine_terminal(
            TerminalRefinementInput {
                terminal_unit: UnitId::new(42).expect("valid unit id"),
                terminal_geometry: &terminal,
                resolved_outlet: GeoCoord::new(10.0, 10.0),
                snap_threshold: pourpoint_core::algo::SnapThreshold::new(1),
            },
            &D8RefinementPantry {
                session: &session,
                raster_source: Some(&source),
            },
        )
        .expect("projected carve should succeed");

    let TerminalRefinementDecision::Applied {
        refined_outlet,
        geometry,
        ..
    } = decision
    else {
        panic!("projected carve should apply");
    };
    let expected_refined_outlet = GeoCoord::new(10.0, 9.999999999999988);
    assert!(
        (refined_outlet.lon - expected_refined_outlet.lon).abs()
            <= INVERSE_PROJECTION_TOLERANCE_DEGREES
            && (refined_outlet.lat - expected_refined_outlet.lat).abs()
                <= INVERSE_PROJECTION_TOLERANCE_DEGREES,
        "inverse-projected refined outlet should be {expected_refined_outlet:?} within \
         {INVERSE_PROJECTION_TOLERANCE_DEGREES} degrees; got {refined_outlet:?}"
    );
    let expected_bbox = Rect::new(
        coord! { x: 951_078.944_848_778, y: 1_281_580.009_108_443_3 },
        coord! { x: 951_117.540_068_018_6, y: 1_281_631.011_053_289_8 },
    );
    assert_eq!(
        source
            .requests
            .lock()
            .expect("request capture should lock")
            .as_slice(),
        &[expected_bbox, expected_bbox]
    );
    let exterior = &geometry.polygon().0[0].exterior().0;
    let expected_corners = [
        (9.999843992312218, 10.000117642583314),
        (10.000159417194476, 10.000117642583314),
        (10.00015600765485, 9.999882357438091),
        (9.999840582880138, 9.999882357438091),
    ];
    for expected in expected_corners {
        assert!(
            exterior.iter().any(|coordinate| {
                (coordinate.x - expected.0).abs() <= INVERSE_PROJECTION_TOLERANCE_DEGREES
                    && (coordinate.y - expected.1).abs() <= INVERSE_PROJECTION_TOLERANCE_DEGREES
            }),
            "inverse-projected carved ring should contain {expected:?}; got {exterior:?}"
        );
    }
    assert_eq!(exterior.first(), exterior.last());
}

#[test]
fn projected_refinement_inverse_projects_carved_interior_ring() {
    let (_tmp, root) = copied_fixture();
    write_projected_manifest(&root);
    let mut projected = manifest(&root);
    projected["auxiliary"][0]["metadata"]["flow_acc_units"] = json!("cells");
    projected["auxiliary"][0]["metadata"]["flow_dir_encoding"] = json!("esri");
    write_manifest(&root, projected);
    write_projected_tiff(&root.join("flow_dir.tif"), FarRasterKind::FlowDir);
    write_projected_tiff(&root.join("flow_acc.tif"), FarRasterKind::FlowAcc);
    let session = DatasetSession::open_path(&root).expect("temp fixture should open");
    let terminal = projected_terminal();
    let source = DonutRasterSource;
    let native_outlet = donut_geo().pixel_to_coord(GridCoord::new(0, 0));
    let outlet =
        inverse(Crs::Epsg8857, native_outlet).expect("native donut outlet should inverse-project");

    let decision = D8RasterRefinementStrategy
        .refine_terminal(
            TerminalRefinementInput {
                terminal_unit: UnitId::new(42).expect("valid unit id"),
                terminal_geometry: &terminal,
                resolved_outlet: outlet,
                snap_threshold: pourpoint_core::algo::SnapThreshold::new(1),
            },
            &D8RefinementPantry {
                session: &session,
                raster_source: Some(&source),
            },
        )
        .expect("projected donut carve should succeed");

    let TerminalRefinementDecision::Applied { geometry, .. } = decision else {
        panic!("projected donut carve should apply");
    };
    let interiors = geometry.polygon().0[0].interiors();
    assert_eq!(interiors.len(), 1, "carved polygon should retain its hole");
    let interior = &interiors[0].0;
    let expected_native_corners = [
        NativeCoord::new(951_093.242_455_628, 1_281_610.510_084_815),
        NativeCoord::new(951_103.242_455_628, 1_281_610.510_084_815),
        NativeCoord::new(951_103.242_455_628, 1_281_600.510_084_815),
        NativeCoord::new(951_093.242_455_628, 1_281_600.510_084_815),
    ];
    let expected_geographic_corners = expected_native_corners.map(|native| {
        inverse(Crs::Epsg8857, native).expect("native hole corner should inverse-project")
    });
    for expected in expected_geographic_corners {
        assert!(
            interior.iter().any(|coordinate| {
                (coordinate.x - expected.lon).abs() <= INVERSE_PROJECTION_TOLERANCE_DEGREES
                    && (coordinate.y - expected.lat).abs() <= INVERSE_PROJECTION_TOLERANCE_DEGREES
            }),
            "inverse-projected interior ring should contain ({}, {}); got {interior:?}",
            expected.lon,
            expected.lat
        );
    }
    assert_eq!(
        interior.first(),
        interior.last(),
        "inverse-projected interior ring must preserve closure"
    );
}

#[test]
fn empty_terminal_routes_through_degenerate_refinement_error() {
    let session = DatasetSession::open_path(&fixture_path()).expect("fixture should open");
    let terminal = MultiPolygon::new(vec![]);

    let err = D8RasterRefinementStrategy
        .refine_terminal(
            TerminalRefinementInput {
                terminal_unit: UnitId::new(42).expect("valid unit id"),
                terminal_geometry: &terminal,
                resolved_outlet: GeoCoord::new(2.5, -2.5),
                snap_threshold: pourpoint_core::algo::SnapThreshold::DEFAULT,
            },
            &D8RefinementPantry {
                session: &session,
                raster_source: None,
            },
        )
        .expect_err("empty terminal should fail as a refinement algorithm error");

    assert!(matches!(
        err,
        TerminalRefinementError::Algorithm {
            unit_id: 42,
            source: pourpoint_core::algo::RefinementError::DegenerateTerminalPolygon,
        }
    ));
    let engine_error = EngineError::from(err);
    assert!(matches!(
        engine_error,
        EngineError::Refinement {
            unit_id: 42,
            source: pourpoint_core::algo::RefinementError::DegenerateTerminalPolygon,
        }
    ));
}

#[test]
fn projected_inverse_failure_routes_through_refinement() {
    let (_tmp, root) = copied_fixture();
    write_projected_manifest(&root);
    let mut projected = manifest(&root);
    projected["auxiliary"][0]["metadata"]["flow_acc_units"] = json!("cells");
    write_manifest(&root, projected);
    write_projected_tiff(&root.join("flow_dir.tif"), FarRasterKind::FlowDir);
    write_projected_tiff(&root.join("flow_acc.tif"), FarRasterKind::FlowAcc);
    let session = DatasetSession::open_path(&root).expect("temp fixture should open");
    let terminal = projected_terminal();
    let source = InverseFailureRasterSource;

    let err = D8RasterRefinementStrategy
        .refine_terminal(
            TerminalRefinementInput {
                terminal_unit: UnitId::new(42).expect("valid unit id"),
                terminal_geometry: &terminal,
                resolved_outlet: GeoCoord::new(10.0, 10.0),
                snap_threshold: pourpoint_core::algo::SnapThreshold::DEFAULT,
            },
            &D8RefinementPantry {
                session: &session,
                raster_source: Some(&source),
            },
        )
        .expect_err("out-of-domain carved ring should fail inverse projection");
    assert!(matches!(
        err,
        TerminalRefinementError::Algorithm {
            source: pourpoint_core::algo::RefinementError::InverseProjection {
                epsg: 8857,
                source: ProjectionError::OutOfDomain { .. },
            },
            ..
        }
    ));
    let rendered = err.to_string();
    assert!(rendered.contains("failed to inverse-project refined output from EPSG:8857"));
    let engine_error = EngineError::from(err);
    assert!(matches!(engine_error, EngineError::Refinement { .. }));
}

#[test]
fn refine_off_still_dissolves_whole_terminal_with_legacy_engine_behavior() {
    let session = DatasetSession::open_path(&fixture_path()).expect("fixture should open");
    let engine = Engine::builder(session).build();
    let options = DelineationOptions::default().with_refinement_mode(RefinementMode::Disabled);

    let result = engine
        .delineate(GeoCoord::new(2.5, -2.5), &options)
        .expect("refine-off delineation should still succeed");

    assert_eq!(result.refinement(), &RefinementOutcome::Disabled);
    assert!(!result.geometry().0.is_empty());
    assert!(result.area_km2().as_f64() > 0.0);
}

#[test]
fn missing_declaration_has_exact_three_mode_contract() {
    let (_tmp, root) = copied_fixture();
    remove_d8_aux(&root);
    assert_three_mode_skip(
        &root,
        Some(SyntheticRasterFailure::FileNotFound),
        BestEffortSkipReason::NoD8AuxDeclared,
        BestEffortSkipCategory::Availability,
        |error| {
            matches!(
                error,
                EngineError::D8Selection {
                    unit_id: 1,
                    source: SessionError::MissingRequiredD8Aux,
                }
            )
        },
    );
}

#[test]
fn missing_attached_raster_source_has_exact_three_mode_contract() {
    let (_tmp, root) = copied_fixture();
    assert_three_mode_skip(
        &root,
        None::<SyntheticRasterFailure>,
        BestEffortSkipReason::NoRasterSourceProvided,
        BestEffortSkipCategory::Availability,
        |error| {
            matches!(
                error,
                EngineError::RequiredD8RasterSourceMissing { unit_id: 1 }
            )
        },
    );
}

#[test]
fn no_covering_d8_skips_best_effort_and_stays_fatal_when_required() {
    let (_tmp, root) = copied_fixture();
    write_far_away_tiff(&root.join("flow_dir.tif"), FarRasterKind::FlowDir);
    write_far_away_tiff(&root.join("flow_acc.tif"), FarRasterKind::FlowAcc);
    let expected_selection_error = SessionError::NoCoveringD8Tile {
        candidates: vec![D8NativeCoverageCandidate {
            declaration_index: 0,
            epsg: 4326,
            min_x: 0.0,
            min_y: -5.0,
            max_x: 5.0,
            max_y: 0.0,
        }],
    };
    assert_three_mode_skip(
        &root,
        Some(SyntheticRasterFailure::FileNotFound),
        BestEffortSkipReason::Availability {
            source: BestEffortSkipSource::D8Selection,
            diagnostic: expected_selection_error.to_string(),
        },
        BestEffortSkipCategory::Availability,
        |error| {
            let EngineError::D8Selection {
                unit_id: 1,
                source: SessionError::NoCoveringD8Tile { candidates },
            } = error
            else {
                return false;
            };
            candidates
                == &[D8NativeCoverageCandidate {
                    declaration_index: 0,
                    epsg: 4326,
                    min_x: 0.0,
                    min_y: -5.0,
                    max_x: 5.0,
                    max_y: 0.0,
                }]
        },
    );
}

#[test]
fn terminal_spanning_two_declarations_has_exact_three_mode_contract() {
    let (_tmp, root) = copied_fixture();
    let mut split_manifest = manifest(&root);
    split_manifest["auxiliary"] = json!([
        {
            "schema": "hfx.aux.d8_raster.v2",
            "artifacts": {
                "flow_dir": "left_flow_dir.tif",
                "flow_acc": "left_flow_acc.tif"
            },
            "metadata": {
                "crs": "EPSG:4326",
                "flow_dir_encoding": "esri",
                "flow_acc_units": "cells"
            }
        },
        {
            "schema": "hfx.aux.d8_raster.v2",
            "artifacts": {
                "flow_dir": "right_flow_dir.tif",
                "flow_acc": "right_flow_acc.tif"
            },
            "metadata": {
                "crs": "EPSG:4326",
                "flow_dir_encoding": "esri",
                "flow_acc_units": "cells"
            }
        }
    ]);
    write_manifest(&root, split_manifest);
    write_extent_tiff(
        &root.join("left_flow_dir.tif"),
        FarRasterKind::FlowDir,
        0.0,
        0.0,
        0.5,
        1.0,
    );
    write_extent_tiff(
        &root.join("left_flow_acc.tif"),
        FarRasterKind::FlowAcc,
        0.0,
        0.0,
        0.5,
        1.0,
    );
    write_extent_tiff(
        &root.join("right_flow_dir.tif"),
        FarRasterKind::FlowDir,
        2.5,
        0.0,
        0.5,
        1.0,
    );
    write_extent_tiff(
        &root.join("right_flow_acc.tif"),
        FarRasterKind::FlowAcc,
        2.5,
        0.0,
        0.5,
        1.0,
    );
    let expected_candidates = vec![
        D8NativeCoverageCandidate {
            declaration_index: 0,
            epsg: 4326,
            min_x: 0.0,
            min_y: -5.0,
            max_x: 5.0,
            max_y: 0.0,
        },
        D8NativeCoverageCandidate {
            declaration_index: 1,
            epsg: 4326,
            min_x: 0.0,
            min_y: -5.0,
            max_x: 5.0,
            max_y: 0.0,
        },
    ];
    let expected_selection_error = SessionError::TerminalSpansD8Tiles {
        declaration_indices: vec![0, 1],
        candidates: expected_candidates.clone(),
    };
    assert_three_mode_skip(
        &root,
        Some(SyntheticRasterFailure::FileNotFound),
        BestEffortSkipReason::Availability {
            source: BestEffortSkipSource::D8Selection,
            diagnostic: expected_selection_error.to_string(),
        },
        BestEffortSkipCategory::Availability,
        |error| {
            let EngineError::D8Selection {
                unit_id: 1,
                source:
                    SessionError::TerminalSpansD8Tiles {
                        declaration_indices,
                        candidates,
                    },
            } = error
            else {
                return false;
            };
            declaration_indices == &[0, 1] && candidates == &expected_candidates
        },
    );
}

#[test]
fn truncated_tiff_header_has_exact_three_mode_contract() {
    let (_tmp, root) = copied_fixture();
    fs::write(root.join("flow_dir.tif"), b"II*\0")
        .expect("truncated flow-dir TIFF header should write");
    let selection_error = DatasetSession::open_path(&root)
        .expect("fixture should open")
        .select_d8_raster_for_terminal(&rect_terminal(synthetic_full_extent()))
        .expect_err("truncated header should fail selection");
    let diagnostic = selection_error.to_string();
    assert_three_mode_skip(
        &root,
        Some(SyntheticRasterFailure::FileNotFound),
        BestEffortSkipReason::Availability {
            source: BestEffortSkipSource::D8Selection,
            diagnostic,
        },
        BestEffortSkipCategory::Availability,
        |error| {
            matches!(
                error,
                EngineError::D8Selection {
                    unit_id: 1,
                    source: SessionError::CogExtentHeaderRead {
                        declaration_index: 0,
                        kind: RasterKind::FlowDir,
                        path,
                        source: CacheError::Tiff { .. },
                    },
                } if path.ends_with("flow_dir.tif")
            )
        },
    );
}

#[test]
fn raster_source_failures_have_exact_three_mode_contracts() {
    let root = fixture_path();
    let expected_uri = root.join("flow_dir.tif").to_string_lossy().into_owned();
    for failure in [
        SyntheticRasterFailure::FileNotFound,
        SyntheticRasterFailure::OpenFailed,
        SyntheticRasterFailure::ReadFailed,
        SyntheticRasterFailure::EmptyWindow,
        SyntheticRasterFailure::TileConstruction,
    ] {
        let expected_raster_error = failure.error(&expected_uri);
        let (expected_reason, expected_category) = match failure {
            SyntheticRasterFailure::TileConstruction => (
                BestEffortSkipReason::DataGeometryIntegrity {
                    source: BestEffortSkipSource::RasterLoad,
                    diagnostic: expected_raster_error.to_string(),
                },
                BestEffortSkipCategory::DataGeometryIntegrity,
            ),
            _ => (
                BestEffortSkipReason::Availability {
                    source: BestEffortSkipSource::RasterLoad,
                    diagnostic: expected_raster_error.to_string(),
                },
                BestEffortSkipCategory::Availability,
            ),
        };
        assert_three_mode_skip(
            &root,
            Some(failure),
            expected_reason,
            expected_category,
            |error| {
                let EngineError::Refinement {
                    unit_id: 1,
                    source: RefinementError::RasterLoad { source },
                } = error
                else {
                    return false;
                };
                match (failure, source) {
                    (
                        SyntheticRasterFailure::FileNotFound,
                        RasterSourceError::FileNotFound { path },
                    ) => path == &expected_uri,
                    (
                        SyntheticRasterFailure::OpenFailed,
                        RasterSourceError::OpenFailed { path, reason },
                    ) => path == &expected_uri && reason == "reader unavailable",
                    (
                        SyntheticRasterFailure::ReadFailed,
                        RasterSourceError::ReadFailed { path, reason },
                    ) => path == &expected_uri && reason == "truncated window",
                    (
                        SyntheticRasterFailure::EmptyWindow,
                        RasterSourceError::EmptyWindow { path },
                    ) => path == &expected_uri,
                    (
                        SyntheticRasterFailure::TileConstruction,
                        RasterSourceError::TileConstruction { reason },
                    ) => reason == "wrong-length buffer after successful read",
                    _ => false,
                }
            },
        );
    }
}

#[test]
fn unsupported_d8_crs_skips_best_effort_and_stays_fatal_when_required() {
    let (_tmp, root) = copied_fixture();
    let mut fixture_manifest = manifest(&root);
    fixture_manifest["auxiliary"][0]["metadata"]["crs"] = json!("EPSG:3857");
    write_manifest(&root, fixture_manifest);
    let expected_selection_error = SessionError::UnsupportedD8Crs {
        declared_crs: "EPSG:3857".to_string(),
        source: ProjectionError::UnsupportedCrs { epsg: 3857 },
    };
    assert_three_mode_skip(
        &root,
        Some(SyntheticRasterFailure::FileNotFound),
        BestEffortSkipReason::MisDeclaration {
            source: BestEffortSkipSource::D8Selection,
            diagnostic: expected_selection_error.to_string(),
        },
        BestEffortSkipCategory::MisDeclaration,
        |error| {
            matches!(
                error,
                EngineError::D8Selection {
                    unit_id: 1,
                    source: SessionError::UnsupportedD8Crs {
                        declared_crs,
                        source: ProjectionError::UnsupportedCrs { epsg: 3857 },
                    },
                } if declared_crs == "EPSG:3857"
            )
        },
    );
}

#[test]
fn out_of_range_d8_crs_has_exact_three_mode_contract() {
    let (_tmp, root) = copied_fixture();
    let mut fixture_manifest = manifest(&root);
    fixture_manifest["auxiliary"][0]["metadata"]["crs"] = json!("EPSG:99999999999");
    write_manifest(&root, fixture_manifest);
    let selection_error = DatasetSession::open_path(&root)
        .expect("fixture should open")
        .select_d8_raster_for_terminal(&rect_terminal(synthetic_full_extent()))
        .expect_err("out-of-range CRS should fail selection");
    assert!(matches!(
        &selection_error,
        SessionError::D8CrsIdentifierOutOfRange {
            declared_crs,
            source: _,
        } if declared_crs == "EPSG:99999999999"
    ));
    assert_three_mode_skip(
        &root,
        Some(SyntheticRasterFailure::FileNotFound),
        BestEffortSkipReason::MisDeclaration {
            source: BestEffortSkipSource::D8Selection,
            diagnostic: selection_error.to_string(),
        },
        BestEffortSkipCategory::MisDeclaration,
        |error| {
            matches!(
                error,
                EngineError::D8Selection {
                    unit_id: 1,
                    source: SessionError::D8CrsIdentifierOutOfRange {
                        declared_crs,
                        source: _,
                    },
                } if declared_crs == "EPSG:99999999999"
            )
        },
    );
}

#[test]
fn geographic_km2_skips_best_effort_and_stays_fatal_when_required() {
    let (_tmp, root) = copied_fixture();
    let mut fixture_manifest = manifest(&root);
    fixture_manifest["auxiliary"][0]["metadata"]["crs"] = json!("EPSG:4326");
    fixture_manifest["auxiliary"][0]["metadata"]["flow_acc_units"] = json!("km2");
    write_manifest(&root, fixture_manifest);
    let expected_refinement_error = RefinementError::GeographicKm2Unsupported {
        epsg: 4326,
        units: FlowAccumulationUnits::Km2,
    };
    assert_three_mode_skip(
        &root,
        Some(LocalTiffRasterSource),
        BestEffortSkipReason::MisDeclaration {
            source: BestEffortSkipSource::RefinementAlgorithm,
            diagnostic: expected_refinement_error.to_string(),
        },
        BestEffortSkipCategory::MisDeclaration,
        |error| {
            matches!(
                error,
                EngineError::Refinement {
                    unit_id: 1,
                    source: RefinementError::GeographicKm2Unsupported {
                        epsg: 4326,
                        units: FlowAccumulationUnits::Km2,
                    },
                }
            )
        },
    );
}

#[test]
fn dimension_mismatch_skips_best_effort_and_stays_fatal_when_required() {
    let expected_refinement_error = RefinementError::DimensionMismatch {
        fd_rows: 5,
        fd_cols: 5,
        acc_rows: 5,
        acc_cols: 4,
    };
    assert_three_mode_skip(
        &fixture_path(),
        Some(DimensionMismatchRasterSource),
        BestEffortSkipReason::MisDeclaration {
            source: BestEffortSkipSource::RefinementAlgorithm,
            diagnostic: expected_refinement_error.to_string(),
        },
        BestEffortSkipCategory::MisDeclaration,
        |error| {
            matches!(
                error,
                EngineError::Refinement {
                    unit_id: 1,
                    source: RefinementError::DimensionMismatch {
                        fd_rows: 5,
                        fd_cols: 5,
                        acc_rows: 5,
                        acc_cols: 4,
                    },
                }
            )
        },
    );
}

#[test]
fn geo_transform_mismatch_skips_best_effort_and_stays_fatal_when_required() {
    let expected_refinement_error = RefinementError::GeoTransformMismatch { rows: 5, cols: 5 };
    assert_three_mode_skip(
        &fixture_path(),
        Some(GeoTransformMismatchRasterSource),
        BestEffortSkipReason::MisDeclaration {
            source: BestEffortSkipSource::RefinementAlgorithm,
            diagnostic: expected_refinement_error.to_string(),
        },
        BestEffortSkipCategory::MisDeclaration,
        |error| {
            matches!(
                error,
                EngineError::Refinement {
                    unit_id: 1,
                    source: RefinementError::GeoTransformMismatch { rows: 5, cols: 5 },
                }
            )
        },
    );
}

#[test]
fn directional_nodata_skips_best_effort_and_stays_fatal_when_required() {
    let expected_raster_error = RasterSourceError::InvalidFlowDirectionNodata {
        nodata: 1,
        encoding: FlowDirEncoding::Esri,
    };
    assert_three_mode_skip(
        &fixture_path(),
        Some(DirectionalNodataRasterSource),
        BestEffortSkipReason::MisDeclaration {
            source: BestEffortSkipSource::RasterLoad,
            diagnostic: expected_raster_error.to_string(),
        },
        BestEffortSkipCategory::MisDeclaration,
        |error| {
            matches!(
                error,
                EngineError::Refinement {
                    unit_id: 1,
                    source: RefinementError::RasterLoad {
                        source: RasterSourceError::InvalidFlowDirectionNodata {
                            nodata: 1,
                            encoding: FlowDirEncoding::Esri,
                        },
                    },
                }
            )
        },
    );
}

#[test]
fn snap_failure_skips_best_effort_and_stays_fatal_when_required() {
    let expected_refinement_error = RefinementError::SnapFailed {
        source: SnapError::NoCellAboveThreshold {
            threshold: 1_000.0,
            units: FlowAccumulationUnits::Cells,
            epsg: 4326,
            outlet_x: 2.5,
            outlet_y: -2.5,
        },
    };
    assert_three_mode_skip(
        &fixture_path(),
        Some(SnapFailureRasterSource),
        BestEffortSkipReason::DataGeometryIntegrity {
            source: BestEffortSkipSource::RefinementAlgorithm,
            diagnostic: expected_refinement_error.to_string(),
        },
        BestEffortSkipCategory::DataGeometryIntegrity,
        |error| {
            matches!(
                error,
                EngineError::Refinement {
                    unit_id: 1,
                    source: RefinementError::SnapFailed {
                        source: SnapError::NoCellAboveThreshold {
                            threshold: 1_000.0,
                            units: FlowAccumulationUnits::Cells,
                            epsg: 4326,
                            outlet_x: 2.5,
                            outlet_y: -2.5,
                        },
                    },
                }
            )
        },
    );
}

#[test]
fn inverse_projection_skips_best_effort_and_stays_fatal_when_required() {
    let (_tmp, root) = copied_fixture();
    write_projected_manifest(&root);
    let mut fixture_manifest = manifest(&root);
    fixture_manifest["auxiliary"][0]["metadata"]["flow_acc_units"] = json!("cells");
    write_manifest(&root, fixture_manifest);
    write_fixture_outlet_projected_tiff(&root.join("flow_dir.tif"), FarRasterKind::FlowDir);
    write_fixture_outlet_projected_tiff(&root.join("flow_acc.tif"), FarRasterKind::FlowAcc);
    let expected_refinement_error = RefinementError::InverseProjection {
        epsg: 8857,
        source: ProjectionError::OutOfDomain {
            x: -99_760_608.494_401_19,
            y: 99_678_832.232_664_75,
        },
    };
    assert_three_mode_skip(
        &root,
        Some(FixtureOutletInverseFailureRasterSource),
        BestEffortSkipReason::DataGeometryIntegrity {
            source: BestEffortSkipSource::RefinementAlgorithm,
            diagnostic: expected_refinement_error.to_string(),
        },
        BestEffortSkipCategory::DataGeometryIntegrity,
        |error| {
            matches!(
                error,
                EngineError::Refinement {
                    unit_id: 1,
                    source: RefinementError::InverseProjection {
                        epsg: 8857,
                        source: ProjectionError::OutOfDomain { x, y },
                    },
                } if *x == -99_760_608.494_401_19 && *y == 99_678_832.232_664_75
            )
        },
    );
}

fn delineate_with_optional_source<R>(
    root: &Path,
    mode: RefinementMode,
    raster_source: Option<R>,
) -> Result<pourpoint_core::DelineationResult, EngineError>
where
    R: RasterSource + Send + Sync + 'static,
{
    let session = DatasetSession::open_path(root).expect("fixture should open");
    let builder = Engine::builder(session);
    let engine = match raster_source {
        Some(source) => builder.with_raster_source(source).build(),
        None => builder.build(),
    };
    engine.delineate(
        GeoCoord::new(2.5, -2.5),
        &DelineationOptions::default().with_refinement_mode(mode),
    )
}

fn assert_three_mode_skip<R>(
    root: &Path,
    raster_source: Option<R>,
    expected_reason: BestEffortSkipReason,
    expected_category: BestEffortSkipCategory,
    required_matches: impl FnOnce(&EngineError) -> bool,
) where
    R: RasterSource + Send + Sync + Clone + 'static,
{
    assert_eq!(expected_reason.category(), expected_category);
    let best_effort =
        delineate_with_optional_source(root, RefinementMode::BestEffort, raster_source.clone())
            .expect("BestEffort should return a typed skip");
    let disabled =
        delineate_with_optional_source(root, RefinementMode::Disabled, raster_source.clone())
            .expect("Disabled should succeed");
    let required_error =
        delineate_with_optional_source(root, RefinementMode::RequireD8, raster_source)
            .expect_err("RequireD8 should retain the original error");
    assert_best_effort_skip_and_disabled_geometry(&best_effort, &disabled, expected_reason);
    assert!(
        required_matches(&required_error),
        "unexpected RequireD8 error: {required_error:?}"
    );
}

fn assert_best_effort_skip_and_disabled_geometry(
    best_effort: &pourpoint_core::DelineationResult,
    disabled: &pourpoint_core::DelineationResult,
    expected_reason: BestEffortSkipReason,
) {
    let RefinementOutcome::BestEffortSkipped { provenance } = best_effort.refinement() else {
        panic!(
            "BestEffort should return a typed skip, got {:?}",
            best_effort.refinement()
        );
    };
    let RefinementProvenance::BestEffortSkipped { strategy, why } = provenance else {
        panic!("BestEffortSkipped outcome should carry skipped provenance");
    };
    assert_eq!(*strategy, RefinementStrategyName::BestEffortD8IfPresent);
    assert_eq!(why, &expected_reason);
    assert_eq!(why.category(), expected_reason.category());
    assert_eq!(
        canonical_wkb_multi_polygon(best_effort.geometry())
            .expect("BestEffort geometry should canonicalize"),
        canonical_wkb_multi_polygon(disabled.geometry())
            .expect("Disabled geometry should canonicalize"),
        "BestEffort skip must preserve the same whole-terminal geometry as Disabled"
    );
}

fn fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_DIR)
}

fn synthetic_full_extent() -> Rect<f64> {
    Rect::new(coord! { x: 0.0, y: -5.0 }, coord! { x: 5.0, y: 0.0 })
}

fn rect_terminal(rect: Rect<f64>) -> MultiPolygon<f64> {
    MultiPolygon::new(vec![Polygon::new(
        LineString::from(vec![
            (rect.min().x, rect.min().y),
            (rect.max().x, rect.min().y),
            (rect.max().x, rect.max().y),
            (rect.min().x, rect.max().y),
            (rect.min().x, rect.min().y),
        ]),
        vec![],
    )])
}

fn projected_terminal() -> MultiPolygon<f64> {
    MultiPolygon::new(vec![Polygon::new(
        LineString::from(vec![
            (9.9998_f64, 9.9998_f64),
            (10.0002_f64, 9.9998_f64),
            (10.0002_f64, 10.0002_f64),
            (9.9998_f64, 10.0002_f64),
            (9.9998_f64, 9.9998_f64),
        ]),
        vec![],
    )])
}

fn projected_terminal_with_hole() -> MultiPolygon<f64> {
    MultiPolygon::new(vec![Polygon::new(
        projected_terminal().0[0].exterior().clone(),
        vec![LineString::from(vec![
            (9.99995_f64, 9.99995_f64),
            (10.00005_f64, 9.99995_f64),
            (10.00005_f64, 10.00005_f64),
            (9.99995_f64, 10.00005_f64),
            (9.99995_f64, 9.99995_f64),
        ])],
    )])
}

fn copied_fixture() -> (TempDir, PathBuf) {
    let tmp = TempDir::new().expect("tempdir should create");
    let root = tmp.path().join("hfx");
    copy_dir_recursive(&fixture_path(), &root);
    (tmp, root)
}

fn copy_dir_recursive(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).expect("destination directory should create");
    for entry in fs::read_dir(src).expect("source directory should read") {
        let entry = entry.expect("source entry should read");
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path);
        } else {
            fs::copy(&src_path, &dst_path).expect("fixture file should copy");
        }
    }
}

fn manifest(root: &Path) -> Value {
    serde_json::from_slice(&fs::read(root.join("manifest.json")).expect("manifest should read"))
        .expect("manifest should parse")
}

fn write_manifest(root: &Path, manifest: Value) {
    fs::write(root.join("manifest.json"), manifest.to_string()).expect("manifest should write");
}

fn write_projected_manifest(root: &Path) {
    write_manifest(
        root,
        json!({
            "format_version": "0.3.0",
            "fabric_name": "testfabric",
            "crs": "EPSG:4326",
            "topology": "tree",
            "bbox": [0.0, -5.0, 5.0, 0.0],
            "unit_count": 1,
            "created_at": "2026-01-01T00:00:00Z",
            "adapter_version": "test-v1",
            "auxiliary": [
                {
                    "schema": "hfx.aux.d8_raster.v2",
                    "artifacts": {
                        "flow_dir": "flow_dir.tif",
                        "flow_acc": "flow_acc.tif"
                    },
                    "metadata": {
                        "crs": "EPSG:8857",
                        "flow_dir_encoding": "grass",
                        "flow_acc_units": "km2"
                    }
                }
            ]
        }),
    );
}

fn prepend_far_away_d8_decl(root: &Path) {
    let mut manifest = manifest(root);
    let aux = manifest["auxiliary"]
        .as_array_mut()
        .expect("fixture auxiliary should be an array");
    aux.insert(
        0,
        json!({
            "schema": "hfx.aux.d8_raster.v2",
            "artifacts": {
                "flow_dir": "far_flow_dir.tif",
                "flow_acc": "far_flow_acc.tif"
            },
            "metadata": {
                "crs": "EPSG:4326",
                "flow_dir_encoding": "esri",
                "flow_acc_units": "cells"
            }
        }),
    );
    write_manifest(root, manifest);
}

fn duplicate_committed_d8_decl(root: &Path) {
    let mut manifest = manifest(root);
    let aux = manifest["auxiliary"]
        .as_array_mut()
        .expect("fixture auxiliary should be an array");
    let original = aux[0].clone();
    aux.push(original);
    write_manifest(root, manifest);
}

fn remove_d8_aux(root: &Path) {
    let mut manifest = manifest(root);
    manifest["auxiliary"] = Value::Array(vec![]);
    write_manifest(root, manifest);
}

enum FarRasterKind {
    FlowDir,
    FlowAcc,
}

#[derive(Clone, Copy)]
enum SyntheticRasterFailure {
    FileNotFound,
    OpenFailed,
    ReadFailed,
    EmptyWindow,
    TileConstruction,
}

impl SyntheticRasterFailure {
    fn error(self, uri: &str) -> RasterSourceError {
        match self {
            Self::FileNotFound => RasterSourceError::FileNotFound {
                path: uri.to_string(),
            },
            Self::OpenFailed => RasterSourceError::OpenFailed {
                path: uri.to_string(),
                reason: "reader unavailable".to_string(),
            },
            Self::ReadFailed => RasterSourceError::ReadFailed {
                path: uri.to_string(),
                reason: "truncated window".to_string(),
            },
            Self::EmptyWindow => RasterSourceError::EmptyWindow {
                path: uri.to_string(),
            },
            Self::TileConstruction => RasterSourceError::TileConstruction {
                reason: "wrong-length buffer after successful read".to_string(),
            },
        }
    }
}

impl RasterSource for SyntheticRasterFailure {
    fn load_flow_direction(
        &self,
        uri: &str,
        _bbox: &Rect<f64>,
        _encoding: FlowDirEncoding,
    ) -> Result<FlowDirectionTile<Raw>, RasterSourceError> {
        Err(self.error(uri))
    }

    fn load_accumulation(
        &self,
        uri: &str,
        _bbox: &Rect<f64>,
    ) -> Result<AccumulationTile<Raw>, RasterSourceError> {
        Err(self.error(uri))
    }
}

#[derive(Clone, Copy)]
struct DimensionMismatchRasterSource;
#[derive(Clone, Copy)]
struct GeoTransformMismatchRasterSource;
#[derive(Clone, Copy)]
struct DirectionalNodataRasterSource;
#[derive(Clone, Copy)]
struct SnapFailureRasterSource;
#[derive(Clone, Copy)]
struct FixtureOutletInverseFailureRasterSource;

#[derive(Default)]
struct ProjectedRasterSource {
    requests: Mutex<Vec<Rect<f64>>>,
}

struct InverseFailureRasterSource;

struct DonutRasterSource;

fn raw_flow_direction(
    values: Vec<u8>,
    dims: GridDims,
    nodata: u8,
    geo: GeoTransform,
    encoding: FlowDirEncoding,
) -> Result<FlowDirectionTile<Raw>, RasterSourceError> {
    let tile = RasterTile::from_vec(values, dims, nodata, geo)
        .expect("test flow-direction tile should construct");
    FlowDirectionTile::from_raw(tile, encoding).map_err(RasterSourceError::from)
}

fn raw_accumulation(
    values: Vec<f32>,
    dims: GridDims,
    geo: GeoTransform,
) -> Result<AccumulationTile<Raw>, RasterSourceError> {
    let tile = RasterTile::from_vec(values, dims, f32::NAN, geo)
        .expect("test accumulation tile should construct");
    Ok(AccumulationTile::from_raw(tile))
}

macro_rules! raster_source {
    ($source:ty, $flow:expr, $acc:expr) => {
        impl RasterSource for $source {
            fn load_flow_direction(
                &self,
                _uri: &str,
                _bbox: &Rect<f64>,
                encoding: FlowDirEncoding,
            ) -> Result<FlowDirectionTile<Raw>, RasterSourceError> {
                $flow(encoding)
            }

            fn load_accumulation(
                &self,
                _uri: &str,
                _bbox: &Rect<f64>,
            ) -> Result<AccumulationTile<Raw>, RasterSourceError> {
                $acc()
            }
        }
    };
}

raster_source!(
    DimensionMismatchRasterSource,
    |encoding| raw_flow_direction(
        vec![0_u8; 25],
        GridDims::new(5, 5),
        255,
        synthetic_fixture_geo(),
        encoding
    ),
    || raw_accumulation(
        vec![1_000.0_f32; 20],
        GridDims::new(5, 4),
        synthetic_fixture_geo()
    )
);

raster_source!(
    GeoTransformMismatchRasterSource,
    |encoding| raw_flow_direction(
        vec![0_u8; 25],
        GridDims::new(5, 5),
        255,
        synthetic_fixture_geo(),
        encoding
    ),
    || raw_accumulation(
        vec![1_000.0_f32; 25],
        GridDims::new(5, 5),
        GeoTransform::new(NativeCoord::new(1.0, 0.0), 1.0, -1.0)
    )
);

raster_source!(
    DirectionalNodataRasterSource,
    |encoding| raw_flow_direction(
        vec![0_u8; 25],
        GridDims::new(5, 5),
        1,
        synthetic_fixture_geo(),
        encoding
    ),
    || raw_accumulation(
        vec![1_000.0_f32; 25],
        GridDims::new(5, 5),
        synthetic_fixture_geo()
    )
);

impl RasterSource for SnapFailureRasterSource {
    fn load_flow_direction(
        &self,
        _uri: &str,
        _bbox: &Rect<f64>,
        encoding: FlowDirEncoding,
    ) -> Result<FlowDirectionTile<Raw>, RasterSourceError> {
        let tile = RasterTile::from_vec(
            vec![0_u8; 25],
            GridDims::new(5, 5),
            255_u8,
            synthetic_fixture_geo(),
        )
        .expect("snap-failure flow-direction tile should construct");
        FlowDirectionTile::from_raw(tile, encoding).map_err(RasterSourceError::from)
    }

    fn load_accumulation(
        &self,
        _uri: &str,
        _bbox: &Rect<f64>,
    ) -> Result<AccumulationTile<Raw>, RasterSourceError> {
        let tile = RasterTile::from_vec(
            vec![0.0_f32; 25],
            GridDims::new(5, 5),
            f32::NAN,
            synthetic_fixture_geo(),
        )
        .expect("snap-failure accumulation tile should construct");
        Ok(AccumulationTile::from_raw(tile))
    }
}

impl RasterSource for FixtureOutletInverseFailureRasterSource {
    fn load_flow_direction(
        &self,
        _uri: &str,
        _bbox: &Rect<f64>,
        encoding: FlowDirEncoding,
    ) -> Result<FlowDirectionTile<Raw>, RasterSourceError> {
        let tile = RasterTile::from_vec(
            vec![0_u8],
            GridDims::new(1, 1),
            255_u8,
            fixture_outlet_inverse_failure_geo(),
        )
        .expect("fixture-outlet inverse-failure flow-direction tile should construct");
        FlowDirectionTile::from_raw(tile, encoding).map_err(RasterSourceError::from)
    }

    fn load_accumulation(
        &self,
        _uri: &str,
        _bbox: &Rect<f64>,
    ) -> Result<AccumulationTile<Raw>, RasterSourceError> {
        let tile = RasterTile::from_vec(
            vec![1_000.0_f32],
            GridDims::new(1, 1),
            f32::NAN,
            fixture_outlet_inverse_failure_geo(),
        )
        .expect("fixture-outlet inverse-failure accumulation tile should construct");
        Ok(AccumulationTile::from_raw(tile))
    }
}

impl RasterSource for DonutRasterSource {
    fn load_flow_direction(
        &self,
        _uri: &str,
        _bbox: &Rect<f64>,
        encoding: FlowDirEncoding,
    ) -> Result<FlowDirectionTile<Raw>, RasterSourceError> {
        #[rustfmt::skip]
        let values = vec![
            0_u8, 16, 16,
            4,     0, 64,
            1,     1, 64,
        ];
        let tile = RasterTile::from_vec(values, GridDims::new(3, 3), 255_u8, donut_geo())
            .expect("donut flow-direction tile should construct");
        FlowDirectionTile::from_raw(tile, encoding).map_err(RasterSourceError::from)
    }

    fn load_accumulation(
        &self,
        _uri: &str,
        _bbox: &Rect<f64>,
    ) -> Result<AccumulationTile<Raw>, RasterSourceError> {
        let mut values = vec![0.0_f32; 9];
        values[0] = 1.0;
        let tile = RasterTile::from_vec(values, GridDims::new(3, 3), f32::NAN, donut_geo())
            .expect("donut accumulation tile should construct");
        Ok(AccumulationTile::from_raw(tile))
    }
}

impl RasterSource for InverseFailureRasterSource {
    fn load_flow_direction(
        &self,
        _uri: &str,
        _bbox: &Rect<f64>,
        encoding: FlowDirEncoding,
    ) -> Result<FlowDirectionTile<Raw>, RasterSourceError> {
        let tile = RasterTile::from_vec(
            vec![0_u8],
            GridDims::new(1, 1),
            255_u8,
            inverse_failure_geo(),
        )
        .expect("inverse-failure flow-direction tile should construct");
        FlowDirectionTile::from_raw(tile, encoding).map_err(RasterSourceError::from)
    }

    fn load_accumulation(
        &self,
        _uri: &str,
        _bbox: &Rect<f64>,
    ) -> Result<AccumulationTile<Raw>, RasterSourceError> {
        let tile = RasterTile::from_vec(
            vec![1_000.0_f32],
            GridDims::new(1, 1),
            f32::NAN,
            inverse_failure_geo(),
        )
        .expect("inverse-failure accumulation tile should construct");
        Ok(AccumulationTile::from_raw(tile))
    }
}

impl RasterSource for ProjectedRasterSource {
    fn load_flow_direction(
        &self,
        _uri: &str,
        bbox: &Rect<f64>,
        encoding: FlowDirEncoding,
    ) -> Result<FlowDirectionTile<Raw>, RasterSourceError> {
        self.requests
            .lock()
            .expect("request capture should lock")
            .push(*bbox);
        let tile =
            RasterTile::from_vec(vec![0_u8; 25], GridDims::new(5, 5), 255_u8, projected_geo())
                .expect("projected flow-direction tile should construct");
        FlowDirectionTile::from_raw(tile, encoding).map_err(RasterSourceError::from)
    }

    fn load_accumulation(
        &self,
        _uri: &str,
        bbox: &Rect<f64>,
    ) -> Result<AccumulationTile<Raw>, RasterSourceError> {
        self.requests
            .lock()
            .expect("request capture should lock")
            .push(*bbox);
        let mut values = vec![0.0_f32; 25];
        values[12] = 1.0;
        let tile = RasterTile::from_vec(values, GridDims::new(5, 5), f32::NAN, projected_geo())
            .expect("projected accumulation tile should construct");
        Ok(AccumulationTile::from_raw(tile))
    }
}

fn projected_geo() -> GeoTransform {
    GeoTransform::new(
        NativeCoord::new(951_023.242_455_628, 1_281_680.510_084_815),
        30.0,
        -30.0,
    )
}

fn inverse_failure_geo() -> GeoTransform {
    GeoTransform::new(
        NativeCoord::new(
            951_098.242_455_628_f64 - 100_000_000.0_f64,
            1_281_605.510_084_815_f64 + 100_000_000.0_f64,
        ),
        200_000_000.0_f64,
        -200_000_000.0_f64,
    )
}

fn synthetic_fixture_geo() -> GeoTransform {
    let extent = synthetic_full_extent();
    GeoTransform::new(
        NativeCoord::new(extent.min().x, extent.max().y),
        (extent.max().x - extent.min().x) / 5.0,
        -(extent.max().y - extent.min().y) / 5.0,
    )
}

fn fixture_outlet_inverse_failure_geo() -> GeoTransform {
    GeoTransform::new(
        NativeCoord::new(
            239_391.505_598_817_37 - 100_000_000.0,
            -321_167.767_335_249_4 + 100_000_000.0,
        ),
        200_000_000.0,
        -200_000_000.0,
    )
}

fn donut_geo() -> GeoTransform {
    GeoTransform::new(
        NativeCoord::new(951_083.242_455_628, 1_281_620.510_084_815),
        10.0,
        -10.0,
    )
}

fn write_far_away_tiff(path: &Path, kind: FarRasterKind) {
    let file = fs::File::create(path).expect("far TIFF should create");
    let mut encoder = TiffEncoder::new(file).expect("TIFF encoder should create");
    match kind {
        FarRasterKind::FlowDir => {
            let mut image = encoder
                .new_image::<colortype::Gray8>(5, 5)
                .expect("flow-dir image should create");
            write_geotiff_tags(&mut image);
            image
                .write_data(&[1_u8; 25])
                .expect("flow-dir image should write");
        }
        FarRasterKind::FlowAcc => {
            let mut image = encoder
                .new_image::<colortype::Gray32Float>(5, 5)
                .expect("flow-acc image should create");
            write_geotiff_tags(&mut image);
            image
                .write_data(&[1.0_f32; 25])
                .expect("flow-acc image should write");
        }
    }
}

fn write_extent_tiff(
    path: &Path,
    kind: FarRasterKind,
    origin_x: f64,
    origin_y: f64,
    pixel_width: f64,
    pixel_height: f64,
) {
    let file = fs::File::create(path).expect("extent TIFF should create");
    let mut encoder = TiffEncoder::new(file).expect("TIFF encoder should create");
    match kind {
        FarRasterKind::FlowDir => {
            let mut image = encoder
                .new_image::<colortype::Gray8>(5, 5)
                .expect("flow-dir image should create");
            write_extent_geotiff_tags(&mut image, origin_x, origin_y, pixel_width, pixel_height);
            image
                .write_data(&[0_u8; 25])
                .expect("flow-dir image should write");
        }
        FarRasterKind::FlowAcc => {
            let mut image = encoder
                .new_image::<colortype::Gray32Float>(5, 5)
                .expect("flow-acc image should create");
            write_extent_geotiff_tags(&mut image, origin_x, origin_y, pixel_width, pixel_height);
            image
                .write_data(&[1_000.0_f32; 25])
                .expect("flow-acc image should write");
        }
    }
}

fn write_extent_geotiff_tags<C, K>(
    image: &mut tiff::encoder::ImageEncoder<'_, fs::File, C, K>,
    origin_x: f64,
    origin_y: f64,
    pixel_width: f64,
    pixel_height: f64,
) where
    C: colortype::ColorType,
    K: tiff::encoder::TiffKind,
{
    let pixel_scale = [pixel_width, pixel_height, 0.0_f64];
    let tiepoint = [0.0_f64, 0.0_f64, 0.0_f64, origin_x, origin_y, 0.0_f64];
    image
        .encoder()
        .write_tag(Tag::ModelPixelScaleTag, &pixel_scale[..])
        .expect("pixel scale tag should write");
    image
        .encoder()
        .write_tag(Tag::ModelTiepointTag, &tiepoint[..])
        .expect("tiepoint tag should write");
}

fn write_projected_tiff(path: &Path, kind: FarRasterKind) {
    let file = fs::File::create(path).expect("projected TIFF should create");
    let mut encoder = TiffEncoder::new(file).expect("TIFF encoder should create");
    match kind {
        FarRasterKind::FlowDir => {
            let mut image = encoder
                .new_image::<colortype::Gray8>(5, 5)
                .expect("flow-dir image should create");
            write_projected_geotiff_tags(&mut image);
            image
                .write_data(&[
                    0_u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                ])
                .expect("flow-dir image should write");
        }
        FarRasterKind::FlowAcc => {
            let mut image = encoder
                .new_image::<colortype::Gray32Float>(5, 5)
                .expect("flow-acc image should create");
            write_projected_geotiff_tags(&mut image);
            image
                .write_data(&[
                    0.0_f32, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0,
                    0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
                ])
                .expect("flow-acc image should write");
        }
    }
}

fn write_fixture_outlet_projected_tiff(path: &Path, kind: FarRasterKind) {
    let file = fs::File::create(path).expect("projected TIFF should create");
    let mut encoder = TiffEncoder::new(file).expect("TIFF encoder should create");
    match kind {
        FarRasterKind::FlowDir => {
            let mut image = encoder
                .new_image::<colortype::Gray8>(5, 5)
                .expect("flow-dir image should create");
            write_fixture_outlet_geotiff_tags(&mut image);
            image
                .write_data(&[0_u8; 25])
                .expect("flow-dir image should write");
        }
        FarRasterKind::FlowAcc => {
            let mut image = encoder
                .new_image::<colortype::Gray32Float>(5, 5)
                .expect("flow-acc image should create");
            write_fixture_outlet_geotiff_tags(&mut image);
            image
                .write_data(&[1_000.0_f32; 25])
                .expect("flow-acc image should write");
        }
    }
}

fn write_fixture_outlet_geotiff_tags<C, K>(
    image: &mut tiff::encoder::ImageEncoder<'_, fs::File, C, K>,
) where
    C: colortype::ColorType,
    K: tiff::encoder::TiffKind,
{
    let pixel_scale = [100_000.0_f64, 150_000.0_f64, 0.0_f64];
    let tiepoint = [0.0_f64, 0.0_f64, 0.0_f64, 0.0_f64, 0.0_f64, 0.0_f64];
    image
        .encoder()
        .write_tag(Tag::ModelPixelScaleTag, &pixel_scale[..])
        .expect("pixel scale tag should write");
    image
        .encoder()
        .write_tag(Tag::ModelTiepointTag, &tiepoint[..])
        .expect("tiepoint tag should write");
}

fn write_projected_geotiff_tags<C, K>(image: &mut tiff::encoder::ImageEncoder<'_, fs::File, C, K>)
where
    C: colortype::ColorType,
    K: tiff::encoder::TiffKind,
{
    let pixel_scale = [30.0_f64, 30.0_f64, 0.0_f64];
    let tiepoint = [
        0.0_f64,
        0.0_f64,
        0.0_f64,
        951_023.242_455_628_f64,
        1_281_680.510_084_815_f64,
        0.0_f64,
    ];
    image
        .encoder()
        .write_tag(Tag::ModelPixelScaleTag, &pixel_scale[..])
        .expect("pixel scale tag should write");
    image
        .encoder()
        .write_tag(Tag::ModelTiepointTag, &tiepoint[..])
        .expect("tiepoint tag should write");
}

fn write_geotiff_tags<C, K>(image: &mut tiff::encoder::ImageEncoder<'_, fs::File, C, K>)
where
    C: colortype::ColorType,
    K: tiff::encoder::TiffKind,
{
    let pixel_scale = [1.0, 1.0, 0.0];
    let tiepoint = [0.0, 0.0, 0.0, 100.0, 105.0, 0.0];
    image
        .encoder()
        .write_tag(Tag::ModelPixelScaleTag, &pixel_scale[..])
        .expect("pixel scale tag should write");
    image
        .encoder()
        .write_tag(Tag::ModelTiepointTag, &tiepoint[..])
        .expect("tiepoint tag should write");
}
