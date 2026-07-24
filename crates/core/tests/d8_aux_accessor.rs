use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use geo::{BoundingRect, LineString, MultiPolygon, Polygon, Rect, coord};
use hfx::{EpsgCode, FlowAccumulationUnits, FlowDirEncoding, UnitId};
use pourpoint_core::algo::coord::GeoCoord;
use pourpoint_core::algo::{
    AccumulationTile, Crs, FlowDirectionTile, GeoTransform, GridCoord, GridDims, NativeCoord,
    ProjectionError, RasterSource, RasterSourceError, RasterTile, Raw, canonical_wkb_multi_polygon,
    forward, inverse,
};
use pourpoint_core::refinement::{
    D8RasterRefinementStrategy, D8RefinementPantry, TerminalRefinementDecision,
    TerminalRefinementError, TerminalRefinementInput, TerminalRefinementStrategy,
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
fn require_d8_without_declared_aux_hard_errors_with_schema_name() {
    let (_tmp, root) = copied_fixture();
    remove_d8_aux(&root);
    let session = DatasetSession::open_path(&root).expect("temp fixture without D8 should open");
    let engine = Engine::builder(session).build();
    let options = DelineationOptions::default().with_refinement_mode(RefinementMode::RequireD8);

    let err = engine
        .delineate(GeoCoord::new(2.5, -2.5), &options)
        .expect_err("RequireD8 should fail when no D8 aux is declared");

    assert!(matches!(err, EngineError::D8Selection { .. }));
    assert!(err.to_string().contains("hfx.aux.d8_raster.v2"));
}

#[test]
fn best_effort_without_declared_aux_visibly_skips_and_dissolves_whole_terminal() {
    let (_tmp, root) = copied_fixture();
    remove_d8_aux(&root);
    let best_effort = {
        let session = DatasetSession::open_path(&root).expect("temp fixture should open");
        Engine::builder(session)
            .build()
            .delineate(GeoCoord::new(2.5, -2.5), &DelineationOptions::default())
            .expect("BestEffort with no D8 aux should succeed")
    };
    let disabled = {
        let session = DatasetSession::open_path(&root).expect("temp fixture should reopen");
        Engine::builder(session)
            .build()
            .delineate(
                GeoCoord::new(2.5, -2.5),
                &DelineationOptions::default().with_refinement_mode(RefinementMode::Disabled),
            )
            .expect("Disabled should succeed")
    };

    assert_eq!(
        best_effort.refinement(),
        &RefinementOutcome::BestEffortSkipped {
            provenance: RefinementProvenance::BestEffortSkipped {
                strategy: RefinementStrategyName::BestEffortD8IfPresent,
                why: BestEffortSkipReason::NoD8AuxDeclared,
            },
        }
    );
    assert_eq!(
        canonical_wkb_multi_polygon(best_effort.geometry())
            .expect("BestEffort geometry should canonicalize"),
        canonical_wkb_multi_polygon(disabled.geometry())
            .expect("Disabled geometry should canonicalize")
    );
}

#[test]
fn selected_d8_read_failure_hard_errors_under_best_effort_and_require_d8() {
    for mode in [RefinementMode::BestEffort, RefinementMode::RequireD8] {
        let session = DatasetSession::open_path(&fixture_path()).expect("fixture should open");
        let engine = Engine::builder(session)
            .with_raster_source(FailingRasterSource)
            .build();
        let options = DelineationOptions::default().with_refinement_mode(mode);

        let err = engine
            .delineate(GeoCoord::new(2.5, -2.5), &options)
            .expect_err("selected but unreadable D8 should hard-error");

        assert!(
            matches!(err, EngineError::Refinement { .. }),
            "expected refinement read failure under {mode:?}, got {err:?}"
        );
    }
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

struct FailingRasterSource;

#[derive(Default)]
struct ProjectedRasterSource {
    requests: Mutex<Vec<Rect<f64>>>,
}

struct InverseFailureRasterSource;

struct DonutRasterSource;

impl RasterSource for DonutRasterSource {
    fn load_flow_direction(
        &self,
        _uri: &str,
        _bbox: &Rect<f64>,
    ) -> Result<FlowDirectionTile<Raw>, RasterSourceError> {
        #[rustfmt::skip]
        let values = vec![
            0_u8, 16, 16,
            4,     0, 64,
            1,     1, 64,
        ];
        let tile = RasterTile::from_vec(values, GridDims::new(3, 3), 255_u8, donut_geo())
            .expect("donut flow-direction tile should construct");
        Ok(FlowDirectionTile::from_raw(tile, FlowDirEncoding::Esri))
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
    ) -> Result<FlowDirectionTile<Raw>, RasterSourceError> {
        let tile = RasterTile::from_vec(
            vec![0_u8],
            GridDims::new(1, 1),
            255_u8,
            inverse_failure_geo(),
        )
        .expect("inverse-failure flow-direction tile should construct");
        Ok(FlowDirectionTile::from_raw(tile, FlowDirEncoding::Grass))
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
    ) -> Result<FlowDirectionTile<Raw>, RasterSourceError> {
        self.requests
            .lock()
            .expect("request capture should lock")
            .push(*bbox);
        let tile =
            RasterTile::from_vec(vec![0_u8; 25], GridDims::new(5, 5), 255_u8, projected_geo())
                .expect("projected flow-direction tile should construct");
        Ok(FlowDirectionTile::from_raw(tile, FlowDirEncoding::Grass))
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

fn donut_geo() -> GeoTransform {
    GeoTransform::new(
        NativeCoord::new(951_083.242_455_628, 1_281_620.510_084_815),
        10.0,
        -10.0,
    )
}

impl RasterSource for FailingRasterSource {
    fn load_flow_direction(
        &self,
        uri: &str,
        _bbox: &Rect<f64>,
    ) -> Result<FlowDirectionTile<Raw>, RasterSourceError> {
        Err(RasterSourceError::FileNotFound {
            path: uri.to_string(),
        })
    }

    fn load_accumulation(
        &self,
        uri: &str,
        _bbox: &Rect<f64>,
    ) -> Result<AccumulationTile<Raw>, RasterSourceError> {
        Err(RasterSourceError::FileNotFound {
            path: uri.to_string(),
        })
    }
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
