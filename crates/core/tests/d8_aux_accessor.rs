use std::fs;
use std::path::{Path, PathBuf};

use geo::{Rect, coord};
use hfx::FlowDirEncoding;
use pourpoint_core::algo::coord::GeoCoord;
use pourpoint_core::algo::{
    AccumulationTile, FlowDirectionTile, RasterSource, RasterSourceError, Raw,
    canonical_wkb_multi_polygon,
};
use pourpoint_core::session::{DatasetSession, RasterKind};
use pourpoint_core::{
    BestEffortSkipReason, DelineationOptions, Engine, EngineError, RefinementMode,
    RefinementOutcome, RefinementProvenance, RefinementStrategyName, SessionError,
};
use serde_json::{Value, json};
use tempfile::TempDir;
use tiff::encoder::{TiffEncoder, colortype};
use tiff::tags::Tag;

const FIXTURE_DIR: &str = "tests/fixtures/parity/v021_synthetic_refined";

#[test]
fn declared_d8_accessor_selects_committed_fixture_paths() {
    let root = fixture_path();
    let session = DatasetSession::open_path(&root).expect("fixture should open");
    let bbox = synthetic_full_extent();

    assert!(session.has_d8_aux());
    let handle = session
        .select_d8_raster_for_bbox(bbox)
        .expect("single declared D8 raster should cover fixture bbox");

    assert_eq!(handle.declaration_index(), 0);
    assert_eq!(handle.flow_dir_encoding(), FlowDirEncoding::Esri);
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
    let handle = session
        .select_d8_raster_for_bbox(synthetic_full_extent())
        .expect("second declaration should cover bbox");

    assert_eq!(handle.declaration_index(), 1);
    assert!(handle.flow_dir_uri().ends_with("flow_dir.tif"));
    assert!(handle.flow_acc_uri().ends_with("flow_acc.tif"));
}

#[test]
fn inclusive_containment_accepts_bbox_equal_to_raster_extent() {
    let session = DatasetSession::open_path(&fixture_path()).expect("fixture should open");
    let handle = session
        .select_d8_raster_for_bbox(synthetic_full_extent())
        .expect("bbox equal to raster extent should count as covered");

    assert_eq!(handle.declaration_index(), 0);
}

#[test]
fn multiple_covering_decls_select_manifest_first() {
    // Two declarations fully cover the bbox (the expected case for a per-basin
    // partitioned D8 fabric, where irregular basins have overlapping rectangular
    // extents). hfx.aux.d8_raster.v1 requires overlapping entries to agree in the
    // overlap, so selection collapses to the manifest-first covering declaration
    // rather than erroring.
    let (_tmp, root) = copied_fixture();
    duplicate_committed_d8_decl(&root);
    let session = DatasetSession::open_path(&root).expect("temp fixture should open");

    let handle = session
        .select_d8_raster_for_bbox(synthetic_full_extent())
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
        .select_d8_raster_for_bbox(synthetic_full_extent())
        .expect_err("explicit D8 selection should require D8 aux");

    assert!(matches!(err, SessionError::MissingRequiredD8Aux));
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
    assert!(err.to_string().contains("hfx.aux.d8_raster.v1"));
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

fn prepend_far_away_d8_decl(root: &Path) {
    let mut manifest = manifest(root);
    let aux = manifest["auxiliary"]
        .as_array_mut()
        .expect("fixture auxiliary should be an array");
    aux.insert(
        0,
        json!({
            "schema": "hfx.aux.d8_raster.v1",
            "artifacts": {
                "flow_dir": "far_flow_dir.tif",
                "flow_acc": "far_flow_acc.tif"
            },
            "metadata": {
                "flow_dir_encoding": "esri"
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
