//! GeoPackage round trips preserve source geometries, attributes, and metadata.

use gdal::vector::{Geometry, LayerAccess, OGRwkbGeometryType};
use gdal::{Dataset, DriverManager, Metadata};
use hfx::{SnapId, SnapTarget, StemRole, UnitId, Weight, WkbGeometry};
use pourpoint_core::reader::manifest::SnapDecl;
use pourpoint_core::snap_targets::{SnapBounds, SnapTargetRow, SnapTargetsMetadata};
use pourpoint_gdal::snap_export::{SnapExportError, SnapGeoPackage};

fn metadata() -> SnapTargetsMetadata {
    SnapTargetsMetadata {
        declarations: vec!["reach-stems", "segment-stems"]
            .into_iter()
            .map(|name| SnapDecl {
                name: name.to_owned(),
                description: format!("{name} description"),
                snap: format!("snap/{name}.parquet"),
                references_levels: vec![0, 2],
                weight_semantics: "Producer preference, not area".to_owned(),
            })
            .collect(),
        manifest: serde_json::json!({"attribution": {"author": "River producer"}, "license": "CC-BY-4.0"}),
    }
}

fn row(id: i64, wkt: &str, role: Option<StemRole>) -> SnapTargetRow {
    let target = SnapTarget::new(
        SnapId::new(id).unwrap(),
        UnitId::new(9007199254740993).unwrap(),
        Weight::new(0.125).unwrap(),
        role,
        None,
        WkbGeometry::new(Geometry::from_wkt(wkt).unwrap().wkb().unwrap()).unwrap(),
    );
    SnapTargetRow { target, bbox: None }
}

#[test]
fn mixed_sets_round_trip_complete_geometry_attributes_and_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("targets.gpkg");
    let metadata = metadata();
    let dataset = DriverManager::get_driver_by_name("GPKG")
        .unwrap()
        .create_vector_only(&path)
        .unwrap();
    let mut writer = SnapGeoPackage::new(dataset, &metadata).unwrap();
    let point = row(9007199254740993, "POINT (8.5 47.5)", None);
    let line = row(
        2,
        "LINESTRING (-100 0, 8.5 47.5, 100 0)",
        Some(StemRole::Distributary),
    );
    for set in &metadata.declarations {
        writer.write(set, &point).unwrap();
        writer.write(set, &line).unwrap();
    }
    writer.finish().unwrap();
    let dataset = Dataset::open(&path).unwrap();
    assert_eq!(dataset.layer_count(), 4);
    assert_eq!(
        dataset.metadata_item("HFX_MANIFEST", "").unwrap(),
        metadata.manifest.to_string()
    );
    for set in &metadata.declarations {
        for (suffix, expected, ty) in [
            ("points", &point, OGRwkbGeometryType::wkbPoint),
            ("lines", &line, OGRwkbGeometryType::wkbLineString),
        ] {
            let mut layer = dataset
                .layer_by_name(&format!("{}_{}", set.name, suffix))
                .unwrap();
            assert_eq!(layer.feature_count(), 1);
            assert_eq!(layer.spatial_ref().unwrap().auth_code().unwrap(), 4326);
            assert_eq!(layer.defn().geometry_type(), ty);
            assert_eq!(
                layer.metadata_item("DESCRIPTION", "").unwrap(),
                set.description
            );
            assert_eq!(
                layer.metadata_item("REFERENCES_LEVELS", "").unwrap(),
                "[0,2]"
            );
            assert_eq!(
                layer.metadata_item("WEIGHT_SEMANTICS", "").unwrap(),
                set.weight_semantics
            );
            let feature = layer.features().next().unwrap();
            assert_eq!(
                feature.field_as_integer64(0).unwrap(),
                Some(expected.target.id().get())
            );
            assert_eq!(
                feature.field_as_integer64(1).unwrap(),
                Some(expected.target.unit_id().get())
            );
            assert_eq!(
                feature.field_as_double(2).unwrap(),
                Some(f64::from(expected.target.weight().get()))
            );
            assert_eq!(
                feature.field_as_string(3).unwrap(),
                expected.target.stem_role().map(|role| role.to_string())
            );
            assert_eq!(feature.field_as_string(4).unwrap(), Some(set.name.clone()));
            assert_eq!(
                feature.geometry().unwrap().wkb().unwrap(),
                expected.target.geometry().as_bytes()
            );
        }
    }
}

#[test]
fn empty_selection_keeps_typed_named_layers_and_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("empty.gpkg");
    let metadata = metadata();
    let dataset = DriverManager::get_driver_by_name("GPKG")
        .unwrap()
        .create_vector_only(&path)
        .unwrap();
    SnapGeoPackage::new(dataset, &metadata)
        .unwrap()
        .finish()
        .unwrap();
    let dataset = Dataset::open(&path).unwrap();
    assert_eq!(dataset.layer_count(), 4);
    let names: Vec<_> = dataset
        .layers()
        .map(|layer| {
            assert_eq!(layer.feature_count(), 0);
            layer.name()
        })
        .collect();
    assert_eq!(
        names,
        [
            "reach-stems_points",
            "reach-stems_lines",
            "segment-stems_points",
            "segment-stems_lines"
        ]
    );
    assert_eq!(
        dataset.metadata_item("HFX_MANIFEST", "").unwrap(),
        metadata.manifest.to_string()
    );
}

#[test]
fn unsupported_geometry_is_an_error_not_a_dropped_row() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("invalid.gpkg");
    let metadata = metadata();
    let dataset = DriverManager::get_driver_by_name("GPKG")
        .unwrap()
        .create_vector_only(&path)
        .unwrap();
    let mut writer = SnapGeoPackage::new(dataset, &metadata).unwrap();
    let polygon = row(1, "POLYGON ((0 0, 1 0, 1 1, 0 0))", None);
    assert!(matches!(
        writer.write(&metadata.declarations[0], &polygon),
        Err(SnapExportError::UnsupportedGeometry { .. })
    ));
}

#[test]
fn big_endian_wkb_keeps_source_coordinates() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("big-endian.gpkg");
    let metadata = metadata();
    let dataset = DriverManager::get_driver_by_name("GPKG")
        .unwrap()
        .create_vector_only(&path)
        .unwrap();
    let mut writer = SnapGeoPackage::new(dataset, &metadata).unwrap();
    let mut wkb = vec![0];
    wkb.extend_from_slice(&1_u32.to_be_bytes());
    wkb.extend_from_slice(&8.12345678912345_f64.to_be_bytes());
    wkb.extend_from_slice(&47.98765432198765_f64.to_be_bytes());
    let point = SnapTarget::new(
        SnapId::new(7).unwrap(),
        UnitId::new(9).unwrap(),
        Weight::new(3.25).unwrap(),
        None,
        None,
        WkbGeometry::new(wkb.clone()).unwrap(),
    );
    let point = SnapTargetRow {
        target: point,
        bbox: None,
    };
    writer.write(&metadata.declarations[0], &point).unwrap();
    writer.finish().unwrap();
    let dataset = Dataset::open(&path).unwrap();
    let mut layer = dataset.layer_by_name("reach-stems_points").unwrap();
    let feature = layer.features().next().unwrap();
    assert_eq!(
        feature.geometry().unwrap().get_point(0),
        Geometry::from_wkb(&wkb).unwrap().get_point(0)
    );
}

#[test]
fn original_optional_bounds_round_trip_without_padding_or_inference() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("bounds.gpkg");
    let metadata = metadata();
    let dataset = DriverManager::get_driver_by_name("GPKG")
        .unwrap()
        .create_vector_only(&path)
        .unwrap();
    let mut writer = SnapGeoPackage::new(dataset, &metadata).unwrap();
    let cases = [
        ("POINT (0 0)", None),
        ("POINT (180 90)", Some([180.0, 90.0, 180.0, 90.0])),
        ("POINT (-180 -90)", Some([-180.0, -90.0, -180.0, -90.0])),
        ("POINT (0 0)", Some([-180.0, -90.0, 180.0, 90.0])),
        (
            "POINT (8.5 47.5)",
            Some([8.123456_f32, 47.123455_f32, 8.987654_f32, 47.987656_f32]),
        ),
    ];
    for (index, (wkt, bounds)) in cases.iter().enumerate() {
        let mut point = row(i64::try_from(index + 1).unwrap(), wkt, None);
        point.bbox =
            bounds.map(|[xmin, ymin, xmax, ymax]| SnapBounds::new(xmin, ymin, xmax, ymax).unwrap());
        writer.write(&metadata.declarations[0], &point).unwrap();
    }
    writer.finish().unwrap();
    let dataset = Dataset::open(&path).unwrap();
    let mut layer = dataset.layer_by_name("reach-stems_points").unwrap();
    assert_eq!(layer.feature_count(), cases.len() as u64);
    for (feature, (_, bounds)) in layer.features().zip(cases) {
        for (offset, name) in ["bbox_xmin", "bbox_ymin", "bbox_xmax", "bbox_ymax"]
            .iter()
            .enumerate()
        {
            assert_eq!(feature.field_index(name).unwrap(), 5 + offset);
            assert_eq!(
                feature.field_as_double(5 + offset).unwrap(),
                bounds.map(|bounds| f64::from(bounds[offset]))
            );
        }
    }
}

#[test]
fn reserved_sqlite_and_geopackage_names_have_collision_free_layer_names() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("reserved.gpkg");
    let mut metadata = metadata();
    metadata.declarations[0].name = "sqlite".to_owned();
    metadata.declarations[1].name = "gpkg".to_owned();
    let dataset = DriverManager::get_driver_by_name("GPKG")
        .unwrap()
        .create_vector_only(&path)
        .unwrap();
    let mut writer = SnapGeoPackage::new(dataset, &metadata).unwrap();
    for declaration in &metadata.declarations {
        writer
            .write(declaration, &row(1, "POINT (0 0)", None))
            .unwrap();
    }
    writer.finish().unwrap();
    let dataset = Dataset::open(&path).unwrap();
    assert_eq!(dataset.layer_count(), 4);
    for name in ["sqlite", "gpkg"] {
        let mut points = dataset
            .layer_by_name(&format!("snap_{name}_points"))
            .unwrap();
        assert_eq!(points.feature_count(), 1);
        assert_eq!(
            points
                .features()
                .next()
                .unwrap()
                .field_as_string(4)
                .unwrap()
                .as_deref(),
            Some(name)
        );
        assert_eq!(
            dataset
                .layer_by_name(&format!("snap_{name}_lines"))
                .unwrap()
                .feature_count(),
            0
        );
    }
}
