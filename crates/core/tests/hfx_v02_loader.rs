use std::collections::BTreeSet;
use std::sync::Arc;

use arrow::array::{
    Array, BinaryArray, Int16Array, Int64Array, LargeBinaryArray, LargeListArray, ListArray,
};
use arrow::datatypes::DataType;
use geo::{Geometry, Rect, coord};
use geozero::ToGeo;
use geozero::wkb::Wkb;
use hfx::{EpsgCode, FlowAccumulationUnits, FlowDirEncoding, Topology, WkbGeometry};
use object_store::path::Path as ObjectPath;
use object_store::{ObjectStore, ObjectStoreExt};
use parquet::arrow::ProjectionMask;
use parquet::arrow::async_reader::{ParquetObjectReader, ParquetRecordBatchStreamBuilder};
use parquet::file::statistics::Statistics;
use pourpoint_core::error::SessionError;
use pourpoint_core::reader::manifest::read_manifest_from_bytes;
use pourpoint_core::session::DatasetSession;
use pourpoint_core::source::DatasetSource;
use pourpoint_core::testutil::DatasetBuilder;
use serde_json::{Value, json};

const REAL_GRIT_V200_URL: &str = "https://basin-delineations-public.upstream.tech/grit/2.0.0/";
const REAL_GRIT_QUERY_BBOX: (f32, f32, f32, f32) = (-123.30, 48.30, -123.20, 48.40);

fn read_manifest(root: &std::path::Path) -> Value {
    serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap()
}

fn write_manifest(root: &std::path::Path, manifest: &Value) {
    std::fs::write(root.join("manifest.json"), manifest.to_string()).unwrap();
}

fn push_auxiliary(root: &std::path::Path, aux: Value) {
    let mut manifest = read_manifest(root);
    manifest["auxiliary"].as_array_mut().unwrap().push(aux);
    write_manifest(root, &manifest);
}

fn replace_auxiliary(root: &std::path::Path, aux: Value) {
    let mut manifest = read_manifest(root);
    manifest["auxiliary"] = json!([aux]);
    write_manifest(root, &manifest);
    std::fs::write(root.join("flow_dir.tif"), b"stub").unwrap();
    std::fs::write(root.join("flow_acc.tif"), b"stub").unwrap();
}

fn replace_raster_stubs_with_committed_fixture(root: &std::path::Path) {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/parity/v021_synthetic_refined");
    std::fs::copy(fixture.join("flow_dir.tif"), root.join("flow_dir.tif")).unwrap();
    std::fs::copy(fixture.join("flow_acc.tif"), root.join("flow_acc.tif")).unwrap();
}

#[test]
fn loads_minimal_v021_dataset() {
    let (_dir, root) = DatasetBuilder::new(3).build();

    let session = DatasetSession::open_path(&root).expect("v0.3.0 fixture should load");

    assert_eq!(session.manifest().format_version().to_string(), "0.3.0");
    assert_eq!(session.manifest().unit_count().get(), 3);
    assert_eq!(session.graph().len(), 3);
}

#[test]
fn manifest_v01_rejected_before_missing_v021_fields() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("manifest.json"),
        r#"{"format_version":"0.1","fabric_name":"testfabric"}"#,
    )
    .unwrap();
    std::fs::write(dir.path().join("graph.parquet"), []).unwrap();
    std::fs::write(dir.path().join("catchments.parquet"), []).unwrap();

    let err = DatasetSession::open_path(dir.path()).unwrap_err();
    assert!(matches!(
        err,
        SessionError::UnsupportedFormatVersion { ref found, .. } if found == "0.1"
    ));
}

#[test]
fn manifest_wrong_version_rejected_before_later_required_fields() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("manifest.json"),
        r#"{"format_version":"0.2.1","auxiliary":[{"artifacts":{}}]}"#,
    )
    .unwrap();
    std::fs::write(dir.path().join("graph.parquet"), []).unwrap();
    std::fs::write(dir.path().join("catchments.parquet"), []).unwrap();

    let err = DatasetSession::open_path(dir.path()).unwrap_err();
    assert!(matches!(
        err,
        SessionError::UnsupportedFormatVersion { ref found, .. } if found == "0.2.1"
    ));
}

#[test]
fn manifest_unsupported_crs_is_typed() {
    let (_dir, root) = DatasetBuilder::new(3).build();
    let mut manifest = read_manifest(&root);
    manifest["crs"] = json!("EPSG:3857");
    write_manifest(&root, &manifest);

    let err = DatasetSession::open_path(&root).unwrap_err();
    assert!(matches!(
        err,
        SessionError::UnsupportedCrs { ref found, .. } if found == "EPSG:3857"
    ));
}

#[test]
fn manifest_unit_count_mismatch_is_typed() {
    let (_dir, root) = DatasetBuilder::new(3).build();
    let mut manifest = read_manifest(&root);
    manifest["unit_count"] = json!(4);
    write_manifest(&root, &manifest);

    let err = DatasetSession::open_path(&root).unwrap_err();
    assert!(matches!(
        err,
        SessionError::UnitCountMismatch {
            manifest_count: 4,
            actual_count: 3
        }
    ));
}

#[test]
fn auxiliary_d8_v2_opens_through_session_path() {
    let (_dir, root) = DatasetBuilder::new(3).build();
    replace_auxiliary(
        &root,
        json!({
            "schema": "hfx.aux.d8_raster.v2",
            "artifacts": {
                "flow_dir": "flow_dir.tif",
                "flow_acc": "flow_acc.tif"
            },
            "metadata": {
                "crs": "EPSG:4326",
                "flow_dir_encoding": "esri",
                "flow_acc_units": "cells"
            }
        }),
    );

    let session = DatasetSession::open_path(&root).expect("v2 D8 declaration should open");
    assert!(session.has_d8_aux());
    replace_raster_stubs_with_committed_fixture(&root);
    let handle = session
        .select_d8_raster_for_bbox(Rect::new(
            coord! { x: 0.0, y: -5.0 },
            coord! { x: 5.0, y: 0.0 },
        ))
        .expect("v2 D8 handle should be selected through the session");
    let expected_crs: EpsgCode = "EPSG:4326".parse().unwrap();
    assert_eq!(handle.crs(), &expected_crs);
    assert_eq!(handle.flow_dir_encoding(), FlowDirEncoding::Esri);
    assert_eq!(
        handle.flow_accumulation_units(),
        FlowAccumulationUnits::Cells
    );
}

#[test]
fn auxiliary_d8_v1_is_rejected_during_session_open() {
    let (_dir, root) = DatasetBuilder::new(3).build();
    replace_auxiliary(
        &root,
        json!({
            "schema": "hfx.aux.d8_raster.v1",
            "artifacts": {
                "flow_dir": "flow_dir.tif",
                "flow_acc": "flow_acc.tif"
            },
            "metadata": {
                "flow_dir_encoding": "esri"
            }
        }),
    );

    let error = DatasetSession::open_path(&root).expect_err("v1 D8 declaration should be rejected");
    assert!(matches!(&error, SessionError::UnsupportedD8RasterV1));
    let rendered = error.to_string();
    assert!(rendered.contains("hfx.aux.d8_raster.v1"));
    assert!(rendered.contains("recompile the dataset with a v2-emitting adapter"));
}

#[test]
fn legacy_graph_arrow_is_rejected_without_fallback() {
    let (_dir, root) = DatasetBuilder::new(3).build();
    std::fs::write(root.join("graph.arrow"), b"legacy arrow bytes").unwrap();

    let err = DatasetSession::open_path(&root).unwrap_err();

    assert!(matches!(
        err,
        SessionError::LegacyGraphArrowRejected { ref path }
            if path.ends_with("graph.arrow")
    ));
}

#[test]
fn auxiliary_d8_missing_or_invalid_metadata_is_typed() {
    let cases = [
        (
            "non-object metadata",
            json!({
                "schema": "hfx.aux.d8_raster.v2",
                "artifacts": { "flow_dir": "flow_dir.tif", "flow_acc": "flow_acc.tif" },
                "metadata": "invalid"
            }),
        ),
        (
            "missing crs",
            json!({
                "schema": "hfx.aux.d8_raster.v2",
                "artifacts": { "flow_dir": "flow_dir.tif", "flow_acc": "flow_acc.tif" },
                "metadata": { "flow_dir_encoding": "esri", "flow_acc_units": "cells" }
            }),
        ),
        (
            "missing encoding",
            json!({
                "schema": "hfx.aux.d8_raster.v2",
                "artifacts": { "flow_dir": "flow_dir.tif", "flow_acc": "flow_acc.tif" },
                "metadata": { "crs": "EPSG:4326", "flow_acc_units": "cells" }
            }),
        ),
        (
            "missing accumulation units",
            json!({
                "schema": "hfx.aux.d8_raster.v2",
                "artifacts": { "flow_dir": "flow_dir.tif", "flow_acc": "flow_acc.tif" },
                "metadata": { "crs": "EPSG:4326", "flow_dir_encoding": "esri" }
            }),
        ),
        (
            "non-string crs",
            json!({
                "schema": "hfx.aux.d8_raster.v2",
                "artifacts": { "flow_dir": "flow_dir.tif", "flow_acc": "flow_acc.tif" },
                "metadata": { "crs": 4326, "flow_dir_encoding": "esri", "flow_acc_units": "cells" }
            }),
        ),
        (
            "invalid crs",
            json!({
                "schema": "hfx.aux.d8_raster.v2",
                "artifacts": { "flow_dir": "flow_dir.tif", "flow_acc": "flow_acc.tif" },
                "metadata": { "crs": "epsg:4326", "flow_dir_encoding": "esri", "flow_acc_units": "cells" }
            }),
        ),
        (
            "invalid encoding",
            json!({
                "schema": "hfx.aux.d8_raster.v2",
                "artifacts": { "flow_dir": "flow_dir.tif", "flow_acc": "flow_acc.tif" },
                "metadata": { "crs": "EPSG:4326", "flow_dir_encoding": "bad", "flow_acc_units": "cells" }
            }),
        ),
        (
            "invalid accumulation units",
            json!({
                "schema": "hfx.aux.d8_raster.v2",
                "artifacts": { "flow_dir": "flow_dir.tif", "flow_acc": "flow_acc.tif" },
                "metadata": { "crs": "EPSG:4326", "flow_dir_encoding": "esri", "flow_acc_units": "square_kilometers" }
            }),
        ),
        (
            "additional metadata property",
            json!({
                "schema": "hfx.aux.d8_raster.v2",
                "artifacts": { "flow_dir": "flow_dir.tif", "flow_acc": "flow_acc.tif" },
                "metadata": {
                    "crs": "EPSG:4326",
                    "flow_dir_encoding": "esri",
                    "flow_acc_units": "cells",
                    "dtype": "uint8"
                }
            }),
        ),
        (
            "missing artifact key",
            json!({
                "schema": "hfx.aux.d8_raster.v2",
                "artifacts": { "flow_dir": "flow_dir.tif" },
                "metadata": { "crs": "EPSG:4326", "flow_dir_encoding": "esri", "flow_acc_units": "cells" }
            }),
        ),
    ];

    for (case, aux) in cases {
        let (_dir, root) = DatasetBuilder::new(3).build();
        push_auxiliary(&root, aux);

        let err = DatasetSession::open_path(&root).unwrap_err();
        assert!(
            matches!(
                err,
                SessionError::AuxiliaryDeclParse { ref schema, .. }
                    if schema == "hfx.aux.d8_raster.v2"
            ),
            "{case}: got {err}"
        );
    }
}

#[test]
fn auxiliary_d8_path_escape_is_typed() {
    let (_dir, root) = DatasetBuilder::new(3).build();
    push_auxiliary(
        &root,
        json!({
            "schema": "hfx.aux.d8_raster.v2",
            "artifacts": { "flow_dir": "../flow_dir.tif", "flow_acc": "flow_acc.tif" },
            "metadata": { "crs": "EPSG:4326", "flow_dir_encoding": "esri", "flow_acc_units": "cells" }
        }),
    );

    let err = DatasetSession::open_path(&root).unwrap_err();
    assert!(matches!(
        err,
        SessionError::AuxiliaryPathEscape {
            ref schema,
            ref artifact,
            ..
        } if schema == "hfx.aux.d8_raster.v2" && artifact == "flow_dir"
    ));
}

#[test]
fn auxiliary_d8_declared_but_missing_artifact_is_typed() {
    let (_dir, root) = DatasetBuilder::new(3).build();
    push_auxiliary(
        &root,
        json!({
            "schema": "hfx.aux.d8_raster.v2",
            "artifacts": { "flow_dir": "flow_dir.tif", "flow_acc": "flow_acc.tif" },
            "metadata": { "crs": "EPSG:4326", "flow_dir_encoding": "esri", "flow_acc_units": "cells" }
        }),
    );

    let err = DatasetSession::open_path(&root).unwrap_err();
    assert!(matches!(
        err,
        SessionError::AuxiliaryArtifactMissing {
            ref schema,
            ref artifact,
            ..
        } if schema == "hfx.aux.d8_raster.v2"
            && (artifact == "flow_acc" || artifact == "flow_dir")
    ));
}

#[test]
fn auxiliary_snap_missing_or_invalid_metadata_is_typed() {
    let cases = [
        (
            "missing name",
            json!({
                "schema": "hfx.aux.snap.v2",
                "artifacts": { "snap": "snap.parquet" },
                "metadata": {
                    "description": "Synthetic snap targets.",
                    "references_levels": [0],
                    "weight_semantics": "higher is preferred"
                }
            }),
        ),
        (
            "empty name",
            json!({
                "schema": "hfx.aux.snap.v2",
                "artifacts": { "snap": "snap.parquet" },
                "metadata": {
                    "name": "",
                    "description": "Synthetic snap targets.",
                    "references_levels": [0],
                    "weight_semantics": "higher is preferred"
                }
            }),
        ),
        (
            "missing description",
            json!({
                "schema": "hfx.aux.snap.v2",
                "artifacts": { "snap": "snap.parquet" },
                "metadata": {
                    "name": "test-snap",
                    "references_levels": [0],
                    "weight_semantics": "higher is preferred"
                }
            }),
        ),
        (
            "empty references_levels",
            json!({
                "schema": "hfx.aux.snap.v2",
                "artifacts": { "snap": "snap.parquet" },
                "metadata": {
                    "name": "test-snap",
                    "description": "Synthetic snap targets.",
                    "references_levels": [],
                    "weight_semantics": "higher is preferred"
                }
            }),
        ),
        (
            "negative references_levels",
            json!({
                "schema": "hfx.aux.snap.v2",
                "artifacts": { "snap": "snap.parquet" },
                "metadata": {
                    "name": "test-snap",
                    "description": "Synthetic snap targets.",
                    "references_levels": [-1],
                    "weight_semantics": "higher is preferred"
                }
            }),
        ),
        (
            "non-integer references_levels",
            json!({
                "schema": "hfx.aux.snap.v2",
                "artifacts": { "snap": "snap.parquet" },
                "metadata": {
                    "name": "test-snap",
                    "description": "Synthetic snap targets.",
                    "references_levels": ["0"],
                    "weight_semantics": "higher is preferred"
                }
            }),
        ),
        (
            "missing weight_semantics",
            json!({
                "schema": "hfx.aux.snap.v2",
                "artifacts": { "snap": "snap.parquet" },
                "metadata": {
                    "name": "test-snap",
                    "description": "Synthetic snap targets.",
                    "references_levels": [0]
                }
            }),
        ),
    ];

    for (case, aux) in cases {
        let (_dir, root) = DatasetBuilder::new(3).build();
        push_auxiliary(&root, aux);

        let err = DatasetSession::open_path(&root).unwrap_err();
        assert!(
            matches!(err, SessionError::SnapAuxMetadataInvalid { .. }),
            "{case}: got {err}"
        );
    }
}

#[test]
fn auxiliary_snap_path_escape_is_typed() {
    let (_dir, root) = DatasetBuilder::new(3).build();
    push_auxiliary(
        &root,
        json!({
            "schema": "hfx.aux.snap.v2",
            "artifacts": { "snap": "/tmp/snap.parquet" },
            "metadata": {
                "name": "test-snap",
                "description": "Synthetic snap targets.",
                "references_levels": [0],
                "weight_semantics": "higher is preferred"
            }
        }),
    );

    let err = DatasetSession::open_path(&root).unwrap_err();
    assert!(matches!(
        err,
        SessionError::AuxiliaryPathEscape {
            ref schema,
            ref artifact,
            ..
        } if schema == "hfx.aux.snap.v2" && artifact == "snap"
    ));
}

#[test]
fn auxiliary_generic_reverse_dns_loads_as_uninterpreted_handle() {
    let (_dir, root) = DatasetBuilder::new(3).build();
    let artifact_rel = "extra/custom.bin";
    std::fs::create_dir(root.join("extra")).unwrap();
    std::fs::write(root.join(artifact_rel), b"custom").unwrap();
    push_auxiliary(
        &root,
        json!({
            "schema": "org.example.custom.v1",
            "artifacts": { "data": artifact_rel },
            "metadata": { "name": "not-a-blessed-schema" }
        }),
    );

    let session = DatasetSession::open_path(&root).unwrap();
    let aux = session.auxiliary_declarations();

    assert!(aux.d8_rasters.is_empty());
    assert!(aux.snaps.is_empty());
    assert_eq!(aux.generic.len(), 1);
    assert_eq!(aux.generic[0].schema, "org.example.custom.v1");
    assert_eq!(aux.generic[0].artifacts["data"], artifact_rel);
    assert_eq!(aux.generic[0].metadata["name"], "not-a-blessed-schema");
    assert!(root.join(&aux.generic[0].artifacts["data"]).is_file());
}

#[test]
fn auxiliary_generic_path_escape_is_typed() {
    let (_dir, root) = DatasetBuilder::new(3).build();
    push_auxiliary(
        &root,
        json!({
            "schema": "org.example.custom.v1",
            "artifacts": { "data": "../custom.bin" },
            "metadata": { "name": "not-a-blessed-schema" }
        }),
    );

    let err = DatasetSession::open_path(&root).unwrap_err();
    assert!(matches!(
        err,
        SessionError::AuxiliaryPathEscape {
            ref schema,
            ref artifact,
            ..
        } if schema == "org.example.custom.v1" && artifact == "data"
    ));
}

#[test]
#[ignore = "network-gated GRIT v2.0.0 public R2 loader proof; set POURPOINT_HFX_V02_REAL_R2_LOAD=1"]
fn grit_v200_public_r2_loads_real_v021_multilevel_dag() {
    if std::env::var("POURPOINT_HFX_V02_REAL_R2_LOAD").as_deref() != Ok("1") {
        println!(
            "skipping real GRIT v2.0.0 bounded readiness proof; set POURPOINT_HFX_V02_REAL_R2_LOAD=1 to enable"
        );
        return;
    }

    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime should start");
    runtime.block_on(async {
        let RemoteDataset { store, root } = open_real_grit_source();

        let manifest_path = remote_artifact_path(&root, "manifest.json");
        let manifest_bytes = store
            .get(&manifest_path)
            .await
            .expect("real GRIT manifest should be reachable")
            .bytes()
            .await
            .expect("real GRIT manifest bytes should read");
        let parsed =
            read_manifest_from_bytes(&manifest_bytes).expect("real GRIT manifest should parse");
        let manifest = parsed.manifest;
        let aux = parsed.aux;

        assert_eq!(manifest.format_version().to_string(), "0.3.0");
        assert_eq!(manifest.crs().to_string(), "EPSG:4326");
        assert_eq!(manifest.unit_count().get(), 22_337_300);
        assert_eq!(manifest.topology(), Topology::Dag);
        assert_eq!(aux.snaps.len(), 2);
        assert!(aux.d8_rasters.is_empty());
        assert!(
            aux.snaps
                .iter()
                .all(|decl| decl.snap.starts_with("aux/") && decl.snap.ends_with(".parquet")),
            "real snap aux declarations should point at nested aux parquet artifacts"
        );

        let graph_path = remote_artifact_path(&root, "graph.parquet");
        let graph_proof = bounded_graph_proof(Arc::clone(&store), graph_path).await;
        for column in ["bbox_minx", "bbox_miny", "bbox_maxx", "bbox_maxy"] {
            assert!(
                graph_proof.schema_columns.contains(column),
                "real graph.parquet missing {column}"
            );
        }
        assert!(
            graph_proof.levels_from_row_group_stats.contains(&0),
            "real graph.parquet row-group level statistics must include L0"
        );
        assert!(
            graph_proof.levels_from_row_group_stats.contains(&1),
            "real graph.parquet row-group level statistics must include L1"
        );
        assert!(graph_proof.sample_rows > 0);
        assert!(
            graph_proof.sample_rows <= graph_proof.first_row_group_rows,
            "bounded graph sample should not read past the first row group"
        );
        assert!(
            graph_proof.sample_non_empty_upstream_lists > 0,
            "bounded graph sample should decode at least one non-empty list<int64>"
        );

        let snap_decl = aux
            .snaps
            .iter()
            .find(|decl| decl.references_levels.contains(&1))
            .or_else(|| aux.snaps.first())
            .expect("manifest asserted two snap declarations");
        let snap_path = remote_artifact_path(&root, &snap_decl.snap);
        let snap_proof = bounded_snap_proof(Arc::clone(&store), snap_path).await;
        assert!(snap_proof.sample_rows > 0);
        assert!(
            snap_proof.sample_rows <= snap_proof.first_matching_row_group_rows,
            "bounded snap sample should not read past one matching row group"
        );
        assert!(snap_proof.decoded_wkb_rows > 0);
        assert!(
            snap_proof
                .decoded_geometry_types
                .iter()
                .all(|kind| *kind == "Point" || *kind == "LineString"),
            "snap geometries should decode as Point or LineString"
        );

        println!("format_version={}", manifest.format_version());
        println!("crs={}", manifest.crs());
        println!("unit_count={}", manifest.unit_count().get());
        println!("topology={}", manifest.topology());
        println!("snap_aux_count={}", aux.snaps.len());
        println!("d8_aux_count={}", aux.d8_rasters.len());
        println!("graph_bbox_columns={:?}", graph_proof.schema_columns);
        println!(
            "levels_from=graph.parquet row-group level statistics; levels={:?}",
            graph_proof.levels_from_row_group_stats
        );
        println!(
            "bounded_graph_decode=first_row_group rows_sampled={} non_empty_upstream_lists={}",
            graph_proof.sample_rows, graph_proof.sample_non_empty_upstream_lists
        );
        println!(
            "bounded_snap_decode=one_bbox_matching_row_group artifact={} rows_sampled={} decoded_wkb_rows={} geometry_types={:?}",
            snap_decl.snap,
            snap_proof.sample_rows,
            snap_proof.decoded_wkb_rows,
            snap_proof.decoded_geometry_types
        );
        println!("full_dataset_session_open=false");
    });
}

struct RemoteDataset {
    store: Arc<dyn ObjectStore>,
    root: ObjectPath,
}

struct GraphProof {
    schema_columns: BTreeSet<String>,
    levels_from_row_group_stats: BTreeSet<i16>,
    first_row_group_rows: usize,
    sample_rows: usize,
    sample_non_empty_upstream_lists: usize,
}

struct SnapProof {
    first_matching_row_group_rows: usize,
    sample_rows: usize,
    decoded_wkb_rows: usize,
    decoded_geometry_types: BTreeSet<&'static str>,
}

fn open_real_grit_source() -> RemoteDataset {
    match DatasetSource::parse(REAL_GRIT_V200_URL).expect("public R2 source should parse") {
        DatasetSource::Remote { store, root, .. } => RemoteDataset { store, root },
        DatasetSource::Local(_) => panic!("real GRIT URL should parse as a remote source"),
    }
}

fn remote_artifact_path(root: &ObjectPath, artifact: &str) -> ObjectPath {
    if root.as_ref().is_empty() {
        ObjectPath::from(artifact)
    } else {
        ObjectPath::from(format!(
            "{}/{artifact}",
            root.as_ref().trim_end_matches('/')
        ))
    }
}

async fn bounded_graph_proof(store: Arc<dyn ObjectStore>, path: ObjectPath) -> GraphProof {
    let builder = ParquetRecordBatchStreamBuilder::new(ParquetObjectReader::new(store, path))
        .await
        .expect("real GRIT graph.parquet footer should load");
    let schema = builder.schema();
    let schema_columns = schema
        .fields()
        .iter()
        .map(|field| field.name().to_string())
        .collect::<BTreeSet<_>>();
    let level_col = schema
        .fields()
        .iter()
        .position(|field| field.name() == "level")
        .expect("graph level column should exist");
    let levels_from_row_group_stats = builder
        .metadata()
        .row_groups()
        .iter()
        .flat_map(|row_group| {
            let stats = row_group.column(level_col).statistics();
            [int16_stat_min(stats), int16_stat_max(stats)]
        })
        .flatten()
        .collect::<BTreeSet<_>>();
    let first_row_group_rows = usize::try_from(builder.metadata().row_group(0).num_rows())
        .expect("row group size should fit usize");

    let mut stream = builder
        .with_row_groups(vec![0])
        .with_batch_size(1024)
        .build()
        .expect("real GRIT graph first row group should stream");

    let mut sample_rows = 0usize;
    let mut sample_non_empty_upstream_lists = 0usize;
    let first_batch = stream
        .next_row_group()
        .await
        .expect("real GRIT graph first row group should read")
        .and_then(|reader| reader.into_iter().next());
    if let Some(batch) = first_batch {
        let batch = batch.expect("real GRIT graph batch should decode");
        let ids = batch
            .column_by_name("id")
            .and_then(|column| column.as_any().downcast_ref::<Int64Array>())
            .expect("graph id should decode as Int64");
        let levels = batch
            .column_by_name("level")
            .and_then(|column| column.as_any().downcast_ref::<Int16Array>())
            .expect("graph level should decode as Int16");
        let upstream = batch
            .column_by_name("upstream_ids")
            .expect("graph upstream_ids should decode");

        for row in 0..batch.num_rows() {
            assert!(!ids.is_null(row), "graph sample id should be non-null");
            assert!(
                !levels.is_null(row),
                "graph sample level should be non-null"
            );
            let upstream_len = upstream_list_len(upstream.as_ref(), row);
            if upstream_len > 0 {
                sample_non_empty_upstream_lists += 1;
            }
        }
        sample_rows += batch.num_rows();
    }

    GraphProof {
        schema_columns,
        levels_from_row_group_stats,
        first_row_group_rows,
        sample_rows,
        sample_non_empty_upstream_lists,
    }
}

async fn bounded_snap_proof(store: Arc<dyn ObjectStore>, path: ObjectPath) -> SnapProof {
    let builder = ParquetRecordBatchStreamBuilder::new(ParquetObjectReader::new(store, path))
        .await
        .expect("real GRIT snap aux footer should load");
    let schema = builder.schema();
    for (name, expected) in [
        ("id", DataType::Int64),
        ("unit_id", DataType::Int64),
        ("weight", DataType::Float32),
        ("geometry", DataType::Binary),
        ("bbox_minx", DataType::Float32),
        ("bbox_miny", DataType::Float32),
        ("bbox_maxx", DataType::Float32),
        ("bbox_maxy", DataType::Float32),
    ] {
        let field = schema
            .field_with_name(name)
            .unwrap_or_else(|_| panic!("real GRIT snap aux missing {name}"));
        assert!(
            field.data_type() == &expected
                || (name == "geometry" && field.data_type() == &DataType::LargeBinary),
            "real GRIT snap aux {name} type mismatch"
        );
    }

    let parquet_schema = builder.parquet_schema();
    let bbox_indices = [
        column_index(parquet_schema, "bbox_minx"),
        column_index(parquet_schema, "bbox_miny"),
        column_index(parquet_schema, "bbox_maxx"),
        column_index(parquet_schema, "bbox_maxy"),
    ];
    let matching_row_group = builder
        .metadata()
        .row_groups()
        .iter()
        .enumerate()
        .find_map(|(index, row_group)| {
            let minx = f32_stat_min(row_group.column(bbox_indices[0]).statistics())?;
            let miny = f32_stat_min(row_group.column(bbox_indices[1]).statistics())?;
            let maxx = f32_stat_max(row_group.column(bbox_indices[2]).statistics())?;
            let maxy = f32_stat_max(row_group.column(bbox_indices[3]).statistics())?;
            let intersects = minx <= REAL_GRIT_QUERY_BBOX.2
                && maxx >= REAL_GRIT_QUERY_BBOX.0
                && miny <= REAL_GRIT_QUERY_BBOX.3
                && maxy >= REAL_GRIT_QUERY_BBOX.1;
            intersects.then_some(index)
        })
        .expect("real GRIT snap aux should have a row group intersecting the test bbox");
    let first_matching_row_group_rows =
        usize::try_from(builder.metadata().row_group(matching_row_group).num_rows())
            .expect("row group size should fit usize");

    let projection = ProjectionMask::roots(
        parquet_schema,
        [
            "id",
            "unit_id",
            "weight",
            "geometry",
            "bbox_minx",
            "bbox_miny",
            "bbox_maxx",
            "bbox_maxy",
        ]
        .into_iter()
        .map(|name| column_index(parquet_schema, name))
        .collect::<Vec<_>>(),
    );
    let mut stream = builder
        .with_projection(projection)
        .with_row_groups(vec![matching_row_group])
        .with_batch_size(1024)
        .build()
        .expect("real GRIT snap matching row group should stream");

    let mut sample_rows = 0usize;
    let mut decoded_wkb_rows = 0usize;
    let mut decoded_geometry_types = BTreeSet::new();
    let first_batch = stream
        .next_row_group()
        .await
        .expect("real GRIT snap matching row group should read")
        .and_then(|reader| reader.into_iter().next());
    if let Some(batch) = first_batch {
        let batch = batch.expect("real GRIT snap batch should decode");
        let ids = batch
            .column_by_name("id")
            .and_then(|column| column.as_any().downcast_ref::<Int64Array>())
            .expect("snap id should decode as Int64");
        let unit_ids = batch
            .column_by_name("unit_id")
            .and_then(|column| column.as_any().downcast_ref::<Int64Array>())
            .expect("snap unit_id should decode as Int64");
        let geometry = batch
            .column_by_name("geometry")
            .expect("snap geometry should be projected");

        for row in 0..batch.num_rows() {
            assert!(!ids.is_null(row), "snap sample id should be non-null");
            assert!(
                !unit_ids.is_null(row),
                "snap sample unit_id should be non-null"
            );
            let wkb = wkb_geometry_from_array(geometry.as_ref(), row);
            let decoded = WkbGeometry::new(wkb)
                .map_err(|error| error.to_string())
                .and_then(|wkb| {
                    Wkb(wkb.as_bytes())
                        .to_geo()
                        .map_err(|error| error.to_string())
                })
                .expect("snap sample geometry WKB should decode");
            decoded_wkb_rows += 1;
            decoded_geometry_types.insert(match decoded {
                Geometry::Point(_) => "Point",
                Geometry::LineString(_) => "LineString",
                _ => "Other",
            });
        }
        sample_rows += batch.num_rows();
    }

    SnapProof {
        first_matching_row_group_rows,
        sample_rows,
        decoded_wkb_rows,
        decoded_geometry_types,
    }
}

fn column_index(schema: &parquet::schema::types::SchemaDescriptor, name: &str) -> usize {
    schema
        .columns()
        .iter()
        .position(|column| column.name() == name)
        .unwrap_or_else(|| panic!("missing parquet column {name}"))
}

fn upstream_list_len(column: &dyn Array, row: usize) -> usize {
    if let Some(list) = column.as_any().downcast_ref::<ListArray>() {
        let values = list.value(row);
        return values
            .as_any()
            .downcast_ref::<Int64Array>()
            .expect("graph upstream_ids values should decode as Int64")
            .len();
    }
    if let Some(list) = column.as_any().downcast_ref::<LargeListArray>() {
        let values = list.value(row);
        return values
            .as_any()
            .downcast_ref::<Int64Array>()
            .expect("graph upstream_ids values should decode as Int64")
            .len();
    }
    panic!("graph upstream_ids should decode as List<Int64> or LargeList<Int64>");
}

fn int16_stat_min(stats: Option<&Statistics>) -> Option<i16> {
    match stats? {
        Statistics::Int32(typed) => typed.min_opt().and_then(|value| i16::try_from(*value).ok()),
        _ => None,
    }
}

fn int16_stat_max(stats: Option<&Statistics>) -> Option<i16> {
    match stats? {
        Statistics::Int32(typed) => typed.max_opt().and_then(|value| i16::try_from(*value).ok()),
        _ => None,
    }
}

fn f32_stat_min(stats: Option<&Statistics>) -> Option<f32> {
    match stats? {
        Statistics::Float(typed) => typed.min_opt().copied(),
        _ => None,
    }
}

fn f32_stat_max(stats: Option<&Statistics>) -> Option<f32> {
    match stats? {
        Statistics::Float(typed) => typed.max_opt().copied(),
        _ => None,
    }
}

fn wkb_geometry_from_array(column: &dyn Array, row: usize) -> Vec<u8> {
    if let Some(binary) = column.as_any().downcast_ref::<BinaryArray>() {
        assert!(!binary.is_null(row), "snap geometry should be non-null");
        return binary.value(row).to_vec();
    }
    if let Some(binary) = column.as_any().downcast_ref::<LargeBinaryArray>() {
        assert!(!binary.is_null(row), "snap geometry should be non-null");
        return binary.value(row).to_vec();
    }
    panic!("snap geometry should decode as Binary or LargeBinary");
}
