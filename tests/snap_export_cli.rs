//! CLI snap export exercises the shipped command and GDAL-readable outputs.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use gdal::vector::LayerAccess;
use pourpoint_core::testutil::{
    DatasetBuilder, TestSnapDeclaration, TestSnapGeometry, TestSnapTarget,
};
use tempfile::TempDir;

fn fixture(broken_second_set: bool) -> (TempDir, PathBuf) {
    let row = |id, geometry| TestSnapTarget {
        id,
        catchment_id: 1,
        weight: if broken_second_set && id == 4 {
            -1.0
        } else {
            id as f32
        },
        is_mainstem: true,
        geometry,
    };
    let (dir, root) = DatasetBuilder::new(1)
        .with_custom_snap_declarations(vec![
            TestSnapDeclaration {
                name: "reach-stems".into(),
                path: "reaches.parquet".into(),
                references_levels: vec![0],
                targets: vec![
                    row(1, TestSnapGeometry::Point(1.0, 1.0)),
                    row(2, TestSnapGeometry::LineString(-1.0, 0.5, 2.0, 0.5)),
                    // Bounds touch the query corner, but the diagonal does not.
                    row(3, TestSnapGeometry::LineString(0.8, 2.0, 2.0, 0.8)),
                ],
            },
            TestSnapDeclaration {
                name: "segment-stems".into(),
                path: "segments.parquet".into(),
                references_levels: vec![0],
                targets: vec![row(4, TestSnapGeometry::Point(0.5, 0.5))],
            },
        ])
        .build();
    // Snap export must not depend on these artifacts or scan their rows.
    std::fs::remove_file(root.join("graph.parquet")).unwrap();
    std::fs::remove_file(root.join("catchments.parquet")).unwrap();
    (dir, root)
}

fn command(root: &Path, output: &Path) -> Command {
    let mut command = Command::cargo_bin("pourpoint").unwrap();
    command
        .arg("export-snap")
        .arg("--dataset")
        .arg(root)
        .arg("--output")
        .arg(output);
    command
}

#[test]
fn regional_export_selects_geometry_and_keeps_full_lines_and_sets() {
    let (_dir, root) = fixture(false);
    let output = root.join("regional.gpkg");
    command(&root, &output)
        .args(["--bbox", "0", "0", "1", "1"])
        .assert()
        .success();
    let dataset = gdal::Dataset::open(&output).unwrap();
    assert_eq!(dataset.layer_count(), 4);
    let mut points = dataset.layer_by_name("reach-stems_points").unwrap();
    assert_eq!(points.feature_count(), 1);
    assert_eq!(points.spatial_ref().unwrap().auth_code().unwrap(), 4326);
    let point = points.features().next().unwrap();
    assert_eq!(
        point
            .field_as_integer64(point.field_index("id").unwrap())
            .unwrap(),
        Some(1)
    );
    assert_eq!(
        point
            .field_as_string(point.field_index("snap_set").unwrap())
            .unwrap()
            .as_deref(),
        Some("reach-stems")
    );
    // DatasetBuilder supplies slightly expanded f32 point bounds. Preserve them,
    // rather than replacing them with the geometry's degenerate [1,1,1,1].
    for (field, expected) in [
        ("bbox_xmin", 1.0_f32 - 1e-6_f32),
        ("bbox_ymin", 1.0_f32 - 1e-6_f32),
        ("bbox_xmax", 1.0_f32 + 1e-6_f32),
        ("bbox_ymax", 1.0_f32 + 1e-6_f32),
    ] {
        assert_eq!(
            point
                .field_as_double(point.field_index(field).unwrap())
                .unwrap(),
            Some(f64::from(expected))
        );
    }
    let mut lines = dataset.layer_by_name("reach-stems_lines").unwrap();
    assert_eq!(
        lines.feature_count(),
        1,
        "bbox-only false positive must be excluded"
    );
    let line = lines.features().next().unwrap();
    let geometry = line.geometry().unwrap();
    assert_eq!(geometry.get_point(0), (-1.0, 0.5, 0.0));
    assert_eq!(geometry.get_point(1), (2.0, 0.5, 0.0));
    assert_eq!(
        dataset
            .layer_by_name("segment-stems_points")
            .unwrap()
            .feature_count(),
        1
    );
}

#[test]
fn all_and_named_set_and_empty_selection_are_valid_files() {
    let (_dir, root) = fixture(false);
    let output = root.join("all.gpkg");
    command(&root, &output)
        .args(["--all", "--snap-set", "reach-stems"])
        .assert()
        .success();
    let dataset = gdal::Dataset::open(&output).unwrap();
    assert_eq!(dataset.layer_count(), 2);
    assert_eq!(
        dataset
            .layer_by_name("reach-stems_lines")
            .unwrap()
            .feature_count(),
        2
    );
    drop(dataset);
    command(&root, &output)
        .args(["--bbox", "10", "10", "11", "11"])
        .assert()
        .success();
    let dataset = gdal::Dataset::open(&output).unwrap();
    assert_eq!(dataset.layer_count(), 4);
    for layer in dataset.layers() {
        assert_eq!(layer.feature_count(), 0);
    }
}

#[test]
fn failed_second_set_preserves_destination_and_cleans_temporary_files() {
    // The second file has a valid footer/schema but an invalid row. Failure
    // occurs during streaming, after rows from the first set have been written.
    let (_dir, root) = fixture(true);
    let output = root.join("existing.gpkg");
    std::fs::write(&output, b"existing destination").unwrap();
    command(&root, &output).arg("--all").assert().failure();
    assert_eq!(std::fs::read(&output).unwrap(), b"existing destination");
    let absent = root.join("absent.gpkg");
    command(&root, &absent).arg("--all").assert().failure();
    assert!(!absent.exists());
    assert!(!std::fs::read_dir(&root).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".pourpoint-snap-")
    }));
}

#[test]
fn explicit_scope_and_valid_extent_are_required() {
    let (_dir, root) = fixture(false);
    let output = root.join("out.gpkg");
    for args in [
        vec![],
        vec!["--all", "--bbox", "0", "0", "1", "1"],
        vec!["--bbox", "2", "0", "1", "1"],
        vec!["--bbox", "NaN", "0", "1", "1"],
        vec!["--bbox", "-181", "0", "1", "1"],
        vec!["--all", "--snap-set", "absent"],
    ] {
        command(&root, &output).args(args).assert().failure();
        assert!(!output.exists());
    }
}

#[test]
fn missing_snap_declarations_are_not_an_empty_selection() {
    let (_dir, root) = DatasetBuilder::new(1).build();
    command(&root, &root.join("out.gpkg"))
        .arg("--all")
        .assert()
        .failure();
}
