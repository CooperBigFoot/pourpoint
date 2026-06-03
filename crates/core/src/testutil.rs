//! Test utilities for building synthetic HFX dataset fixtures.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow::array::{
    BinaryBuilder, Float32Builder, Float64Builder, Int16Builder, Int64Array, Int64Builder,
    ListBuilder, RecordBatch, StringBuilder,
};
use arrow::datatypes::{DataType, Field, Schema};
use hfx_core::{Level, UnitId};
use parquet::arrow::ArrowWriter;
use parquet::file::properties::{EnabledStatistics, WriterProperties};
use serde_json::json;
use tempfile::TempDir;

use crate::algo::GeoCoord;

/// Custom catchment specification for outlet resolution tests.
pub struct TestCatchment {
    pub id: i64,
    pub area_km2: f32,
    pub up_area_km2: Option<f32>,
    /// Rectangle polygon as (minx, miny, maxx, maxy).
    pub polygon: (f64, f64, f64, f64),
}

/// Custom snap target specification for outlet resolution tests.
#[derive(Clone)]
pub struct TestSnapTarget {
    pub id: i64,
    pub catchment_id: i64,
    pub weight: f32,
    pub is_mainstem: bool,
    pub geometry: TestSnapGeometry,
}

/// Geometry for a test snap target.
#[derive(Clone)]
pub enum TestSnapGeometry {
    /// A WKB Point at (lon, lat).
    Point(f64, f64),
    /// A WKB LineString from (x1, y1) to (x2, y2).
    LineString(f64, f64, f64, f64),
}

/// Custom snap declaration specification for tests that need multiple snap artifacts.
pub struct TestSnapDeclaration {
    pub name: String,
    pub path: String,
    pub references_levels: Vec<i16>,
    pub targets: Vec<TestSnapTarget>,
}

/// Typed catchment row used by builder-only fixture shapes.
#[derive(Debug, Clone, PartialEq)]
pub struct FixtureUnit {
    id: UnitId,
    level: Level,
    parent_id: Option<UnitId>,
    area_km2: f32,
    up_area_km2: Option<f32>,
    polygon: (f64, f64, f64, f64),
}

impl FixtureUnit {
    /// Parse a fixture unit from boundary literals.
    pub fn new(
        id: i64,
        level: i16,
        parent_id: Option<i64>,
        area_km2: f32,
        up_area_km2: Option<f32>,
        polygon: (f64, f64, f64, f64),
    ) -> Self {
        Self {
            id: UnitId::new(id).unwrap(),
            level: Level::new(level).unwrap(),
            parent_id: parent_id.map(|id| UnitId::new(id).unwrap()),
            area_km2,
            up_area_km2,
            polygon,
        }
    }
}

/// Typed graph row used by builder-only fixture shapes.
#[derive(Debug, Clone, PartialEq)]
pub struct FixtureGraphRow {
    id: UnitId,
    level: Level,
    upstream_ids: Vec<UnitId>,
}

impl FixtureGraphRow {
    /// Parse a fixture graph row from boundary literals.
    pub fn new(id: i64, level: i16, upstream_ids: Vec<i64>) -> Self {
        Self {
            id: UnitId::new(id).unwrap(),
            level: Level::new(level).unwrap(),
            upstream_ids: upstream_ids
                .into_iter()
                .map(|id| UnitId::new(id).unwrap())
                .collect(),
        }
    }
}

/// Builder for synthetic HFX dataset fixtures used in integration tests.
pub struct DatasetBuilder {
    dir: TempDir,
    unit_count: usize,
    topology: &'static str,
    include_snap: bool,
    include_rasters: bool,
    row_group_size: usize,
    polygon_complexity: usize,
    generated_longitude_span: Option<(f64, f64)>,
    dag_diamond: bool,
    multilevel_nested: bool,
    custom_catchments: Option<Vec<TestCatchment>>,
    custom_snap_targets: Option<Vec<TestSnapTarget>>,
    custom_snap_declarations: Option<Vec<TestSnapDeclaration>>,
}

impl DatasetBuilder {
    /// Create a new builder with a linear chain of `unit_count` units.
    pub fn new(unit_count: usize) -> Self {
        Self {
            dir: TempDir::new().unwrap(),
            unit_count,
            topology: "tree",
            include_snap: false,
            include_rasters: false,
            row_group_size: 8192,
            polygon_complexity: 5,
            generated_longitude_span: None,
            dag_diamond: false,
            multilevel_nested: false,
            custom_catchments: None,
            custom_snap_targets: None,
            custom_snap_declarations: None,
        }
    }

    /// Include a `snap.parquet` artifact in the dataset.
    pub fn with_snap(mut self) -> Self {
        self.include_snap = true;
        self
    }

    /// Include stub raster files (`flow_dir.tif`, `flow_acc.tif`) in the dataset.
    pub fn with_rasters(mut self) -> Self {
        self.include_rasters = true;
        self
    }

    /// Set the Parquet row group size for catchments and snap files.
    pub fn with_row_group_size(mut self, size: usize) -> Self {
        self.row_group_size = size;
        self
    }

    /// Set the number of coordinates in generated catchment polygon rings.
    ///
    /// Values up to the default rectangle ring size keep the existing rectangle fixture geometry.
    pub fn with_polygon_complexity(mut self, coords_per_ring: usize) -> Self {
        self.polygon_complexity = coords_per_ring;
        self
    }

    /// Compress auto-generated catchments into the provided longitude span.
    ///
    /// This only affects generated catchments and generated snap targets. Custom
    /// catchments keep their caller-provided coordinates unchanged.
    pub fn with_longitude_span(mut self, min_lon: f64, max_lon: f64) -> Self {
        assert!(
            min_lon.is_finite() && max_lon.is_finite(),
            "longitude span bounds must be finite"
        );
        assert!(
            (-180.0..=180.0).contains(&min_lon) && (-180.0..=180.0).contains(&max_lon),
            "longitude span must fit EPSG:4326 bounds"
        );
        assert!(min_lon < max_lon, "longitude span must be non-empty");
        self.generated_longitude_span = Some((min_lon, max_lon));
        self
    }

    /// Return the center of the terminal generated unit, if this builder uses generated catchments.
    pub fn generated_terminal_unit_center(&self) -> Option<GeoCoord> {
        if self.custom_catchments.is_some() || self.unit_count == 0 {
            return None;
        }

        let unit_count = self.generated_unit_count();
        let (minx, miny, maxx, maxy) = self.generated_unit_bbox(unit_count - 1, unit_count);
        Some(GeoCoord::new(
            ((minx + maxx) / 2.0) as f64,
            ((miny + maxy) / 2.0) as f64,
        ))
    }

    /// Add a diamond bifurcation pattern and mark the topology as DAG.
    pub fn with_dag(mut self) -> Self {
        self.topology = "dag";
        self.dag_diamond = true;
        self.multilevel_nested = false;
        self
    }

    /// Emit a nested L0/L1 fixture with same-level graph edges.
    pub fn with_multilevel_nested(mut self) -> Self {
        self.topology = "tree";
        self.unit_count = nested_fixture_units().len();
        self.dag_diamond = false;
        self.multilevel_nested = true;
        self
    }

    /// Override auto-generated catchments with custom specifications.
    ///
    /// The graph will be built as a linear chain of the provided IDs.
    /// The `unit_count` is automatically set to the number of custom catchments.
    pub fn with_custom_catchments(mut self, catchments: Vec<TestCatchment>) -> Self {
        self.unit_count = catchments.len();
        self.custom_catchments = Some(catchments);
        self
    }

    /// Override auto-generated snap targets with custom specifications.
    ///
    /// Automatically enables the snap artifact.
    pub fn with_custom_snap_targets(mut self, targets: Vec<TestSnapTarget>) -> Self {
        self.include_snap = true;
        self.custom_snap_targets = Some(targets);
        self
    }

    /// Override snap auxiliary declarations and write one snap artifact per declaration.
    ///
    /// This is intentionally narrow test support for HFX v0.2 multi-declaration
    /// snap fixtures; production selection remains in `DatasetSession`.
    pub fn with_custom_snap_declarations(mut self, declarations: Vec<TestSnapDeclaration>) -> Self {
        self.include_snap = true;
        self.custom_snap_declarations = Some(declarations);
        self
    }

    /// Write all artifacts and return `(TempDir, path_to_dataset_root)`.
    ///
    /// The [`TempDir`] must be kept alive by the caller to prevent cleanup.
    pub fn build(self) -> (TempDir, PathBuf) {
        let root = self.dir.path().to_path_buf();
        self.write_manifest(&root);
        self.write_graph(&root);
        self.write_catchments(&root);
        if self.include_snap {
            self.write_snaps(&root);
        }
        if self.include_rasters {
            self.write_raster_stubs(&root);
        }
        (self.dir, root)
    }

    // -----------------------------------------------------------------------
    // Artifact writers
    // -----------------------------------------------------------------------

    fn write_manifest(&self, root: &Path) {
        let unit_count = self.generated_unit_count();
        let mut manifest = json!({
            "format_version": "0.2.1",
            "fabric_name": "testfabric",
            "crs": "EPSG:4326",
            "topology": self.topology,
            "bbox": [-180.0, -90.0, 180.0, 90.0],
            "unit_count": unit_count,
            "created_at": "2026-01-01T00:00:00Z",
            "adapter_version": "test-v1",
            "auxiliary": []
        });
        if self.include_snap {
            let auxiliary = manifest["auxiliary"].as_array_mut().unwrap();
            if let Some(declarations) = &self.custom_snap_declarations {
                for declaration in declarations {
                    auxiliary.push(json!({
                        "schema": "hfx.aux.snap.v1",
                        "artifacts": { "snap": declaration.path },
                        "metadata": {
                            "name": declaration.name,
                            "description": "Synthetic snap targets.",
                            "references_levels": declaration.references_levels,
                            "weight_semantics": "higher is preferred"
                        }
                    }));
                }
            } else {
                auxiliary.push(json!({
                    "schema": "hfx.aux.snap.v1",
                    "artifacts": { "snap": "snap.parquet" },
                    "metadata": {
                        "name": "test-snap",
                        "description": "Synthetic snap targets.",
                        "references_levels": [0],
                        "weight_semantics": "higher is preferred"
                    }
                }));
            }
        }
        if self.include_rasters {
            manifest["auxiliary"].as_array_mut().unwrap().push(json!({
                "schema": "hfx.aux.d8_raster.v1",
                "artifacts": { "flow_dir": "flow_dir.tif", "flow_acc": "flow_acc.tif" },
                "metadata": { "flow_dir_encoding": "esri" }
            }));
        }
        std::fs::write(root.join("manifest.json"), manifest.to_string()).unwrap();
    }

    fn write_graph(&self, root: &Path) {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("level", DataType::Int16, false),
            Field::new(
                "upstream_ids",
                DataType::List(Arc::new(Field::new("item", DataType::Int64, true))),
                false,
            ),
            Field::new("bbox_minx", DataType::Float32, false),
            Field::new("bbox_miny", DataType::Float32, false),
            Field::new("bbox_maxx", DataType::Float32, false),
            Field::new("bbox_maxy", DataType::Float32, false),
        ]));

        let (ids, upstream_ids) = self.build_graph_data();

        let graph_rows = self.fixture_graph_rows();
        let id_arr = if let Some(rows) = &graph_rows {
            Int64Array::from(rows.iter().map(|row| row.id.get()).collect::<Vec<_>>())
        } else {
            Int64Array::from(ids)
        };
        let mut level_b = Int16Builder::new();
        let mut list_builder = ListBuilder::new(Int64Builder::new());
        let mut minx_b = Float32Builder::new();
        let mut miny_b = Float32Builder::new();
        let mut maxx_b = Float32Builder::new();
        let mut maxy_b = Float32Builder::new();
        if let Some(rows) = &graph_rows {
            let units = self.fixture_units().unwrap();
            for row in rows {
                let unit = units.iter().find(|unit| unit.id == row.id).unwrap();
                let (minx, miny, maxx, maxy) = unit.polygon;
                level_b.append_value(row.level.get());
                for upstream_id in &row.upstream_ids {
                    list_builder.values().append_value(upstream_id.get());
                }
                list_builder.append(true);
                minx_b.append_value(minx as f32);
                miny_b.append_value(miny as f32);
                maxx_b.append_value(maxx as f32);
                maxy_b.append_value(maxy as f32);
            }
        } else {
            let generated_unit_count = upstream_ids.len();
            for (idx, ups) in upstream_ids.iter().enumerate() {
                let (minx, miny, maxx, maxy) = self.generated_unit_bbox(idx, generated_unit_count);
                level_b.append_value(0);
                for &u in ups {
                    list_builder.values().append_value(u);
                }
                list_builder.append(true);
                minx_b.append_value(minx);
                miny_b.append_value(miny);
                maxx_b.append_value(maxx);
                maxy_b.append_value(maxy);
            }
        }
        let upstream_arr = list_builder.finish();

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(id_arr),
                Arc::new(level_b.finish()),
                Arc::new(upstream_arr),
                Arc::new(minx_b.finish()),
                Arc::new(miny_b.finish()),
                Arc::new(maxx_b.finish()),
                Arc::new(maxy_b.finish()),
            ],
        )
        .unwrap();

        let props = WriterProperties::builder()
            .set_statistics_enabled(EnabledStatistics::Chunk)
            .build();
        let file = std::fs::File::create(root.join("graph.parquet")).unwrap();
        let mut writer = ArrowWriter::try_new(file, schema, Some(props)).unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();
    }

    fn write_catchments(&self, root: &Path) {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("level", DataType::Int16, false),
            Field::new("parent_id", DataType::Int64, true),
            Field::new("area_km2", DataType::Float32, false),
            Field::new("up_area_km2", DataType::Float32, true),
            Field::new("outlet_lon", DataType::Float64, false),
            Field::new("outlet_lat", DataType::Float64, false),
            Field::new("bbox_minx", DataType::Float32, false),
            Field::new("bbox_miny", DataType::Float32, false),
            Field::new("bbox_maxx", DataType::Float32, false),
            Field::new("bbox_maxy", DataType::Float32, false),
            Field::new("geometry", DataType::Binary, false),
        ]));

        let props = WriterProperties::builder()
            .set_max_row_group_row_count(Some(self.row_group_size))
            .set_statistics_enabled(EnabledStatistics::Chunk)
            .build();

        let file = std::fs::File::create(root.join("catchments.parquet")).unwrap();
        let mut writer = ArrowWriter::try_new(file, schema.clone(), Some(props)).unwrap();

        let mut id_b = Int64Builder::new();
        let mut level_b = Int16Builder::new();
        let mut parent_id_b = Int64Builder::new();
        let mut area_b = Float32Builder::new();
        let mut up_area_b = Float32Builder::new();
        let mut outlet_lon_b = Float64Builder::new();
        let mut outlet_lat_b = Float64Builder::new();
        let mut minx_b = Float32Builder::new();
        let mut miny_b = Float32Builder::new();
        let mut maxx_b = Float32Builder::new();
        let mut maxy_b = Float32Builder::new();
        let mut geom_b = BinaryBuilder::new();

        if let Some(units) = self.fixture_units() {
            for unit in &units {
                let (poly_minx, poly_miny, poly_maxx, poly_maxy) = unit.polygon;
                id_b.append_value(unit.id.get());
                level_b.append_value(unit.level.get());
                match unit.parent_id {
                    Some(parent_id) => parent_id_b.append_value(parent_id.get()),
                    None => parent_id_b.append_null(),
                }
                area_b.append_value(unit.area_km2);
                match unit.up_area_km2 {
                    Some(v) => up_area_b.append_value(v),
                    None => up_area_b.append_null(),
                }
                minx_b.append_value(poly_minx as f32);
                miny_b.append_value(poly_miny as f32);
                maxx_b.append_value(poly_maxx as f32);
                maxy_b.append_value(poly_maxy as f32);
                outlet_lon_b.append_value((poly_minx + poly_maxx) / 2.0);
                outlet_lat_b.append_value((poly_miny + poly_maxy) / 2.0);
                let wkb = minimal_wkb_polygon(poly_minx, poly_miny, poly_maxx, poly_maxy);
                geom_b.append_value(&wkb);
            }
        } else if let Some(customs) = &self.custom_catchments {
            for c in customs {
                let (poly_minx, poly_miny, poly_maxx, poly_maxy) = c.polygon;
                id_b.append_value(c.id);
                level_b.append_value(0);
                parent_id_b.append_null();
                area_b.append_value(c.area_km2);
                match c.up_area_km2 {
                    Some(v) => up_area_b.append_value(v),
                    None => up_area_b.append_null(),
                }
                minx_b.append_value(poly_minx as f32);
                miny_b.append_value(poly_miny as f32);
                maxx_b.append_value(poly_maxx as f32);
                maxy_b.append_value(poly_maxy as f32);
                outlet_lon_b.append_value((poly_minx + poly_maxx) / 2.0);
                outlet_lat_b.append_value((poly_miny + poly_maxy) / 2.0);
                let wkb = minimal_wkb_polygon(poly_minx, poly_miny, poly_maxx, poly_maxy);
                geom_b.append_value(&wkb);
            }
        } else {
            let (ids, _) = self.build_graph_data();
            let generated_unit_count = ids.len();
            for (idx, &id) in ids.iter().enumerate() {
                let (minx, miny, maxx, maxy) = self.generated_unit_bbox(idx, generated_unit_count);

                id_b.append_value(id);
                level_b.append_value(0);
                parent_id_b.append_null();
                area_b.append_value(10.0f32);
                up_area_b.append_null();
                minx_b.append_value(minx);
                miny_b.append_value(miny);
                maxx_b.append_value(maxx);
                maxy_b.append_value(maxy);
                outlet_lon_b.append_value(((minx + maxx) / 2.0) as f64);
                outlet_lat_b.append_value(((miny + maxy) / 2.0) as f64);

                let wkb = generated_wkb_polygon(
                    minx as f64,
                    miny as f64,
                    maxx as f64,
                    maxy as f64,
                    self.polygon_complexity,
                );
                geom_b.append_value(&wkb);
            }
        }

        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(id_b.finish()),
                Arc::new(level_b.finish()),
                Arc::new(parent_id_b.finish()),
                Arc::new(area_b.finish()),
                Arc::new(up_area_b.finish()),
                Arc::new(outlet_lon_b.finish()),
                Arc::new(outlet_lat_b.finish()),
                Arc::new(minx_b.finish()),
                Arc::new(miny_b.finish()),
                Arc::new(maxx_b.finish()),
                Arc::new(maxy_b.finish()),
                Arc::new(geom_b.finish()),
            ],
        )
        .unwrap();

        writer.write(&batch).unwrap();
        writer.close().unwrap();
    }

    fn write_snaps(&self, root: &Path) {
        if let Some(declarations) = &self.custom_snap_declarations {
            for declaration in declarations {
                self.write_snap_artifact(root, &declaration.path, Some(&declaration.targets));
            }
        } else {
            self.write_snap_artifact(root, "snap.parquet", self.custom_snap_targets.as_ref());
        }
    }

    fn write_snap_artifact(
        &self,
        root: &Path,
        path: &str,
        custom_targets: Option<&Vec<TestSnapTarget>>,
    ) {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("unit_id", DataType::Int64, false),
            Field::new("weight", DataType::Float32, false),
            Field::new("stem_role", DataType::Utf8, true),
            Field::new("bbox_minx", DataType::Float32, false),
            Field::new("bbox_miny", DataType::Float32, false),
            Field::new("bbox_maxx", DataType::Float32, false),
            Field::new("bbox_maxy", DataType::Float32, false),
            Field::new("geometry", DataType::Binary, false),
        ]));

        let props = WriterProperties::builder()
            .set_max_row_group_row_count(Some(self.row_group_size))
            .set_statistics_enabled(EnabledStatistics::Chunk)
            .build();

        let file = std::fs::File::create(root.join(path)).unwrap();
        let mut writer = ArrowWriter::try_new(file, schema.clone(), Some(props)).unwrap();

        let mut id_b = Int64Builder::new();
        let mut unit_id_b = Int64Builder::new();
        let mut weight_b = Float32Builder::new();
        let mut stem_role_b = StringBuilder::new();
        let mut minx_b = Float32Builder::new();
        let mut miny_b = Float32Builder::new();
        let mut maxx_b = Float32Builder::new();
        let mut maxy_b = Float32Builder::new();
        let mut geom_b = BinaryBuilder::new();

        if let Some(customs) = custom_targets {
            for t in customs {
                id_b.append_value(t.id);
                unit_id_b.append_value(t.catchment_id);
                weight_b.append_value(t.weight);
                stem_role_b.append_value(if t.is_mainstem {
                    "mainstem"
                } else {
                    "tributary"
                });
                match &t.geometry {
                    TestSnapGeometry::Point(x, y) => {
                        // Point bbox needs non-zero extent.
                        let eps: f32 = 1e-6;
                        minx_b.append_value(*x as f32 - eps);
                        miny_b.append_value(*y as f32 - eps);
                        maxx_b.append_value(*x as f32 + eps);
                        maxy_b.append_value(*y as f32 + eps);
                        let wkb = minimal_wkb_point(*x, *y);
                        geom_b.append_value(&wkb);
                    }
                    TestSnapGeometry::LineString(x1, y1, x2, y2) => {
                        minx_b.append_value(x1.min(*x2) as f32);
                        miny_b.append_value(y1.min(*y2) as f32);
                        maxx_b.append_value(x1.max(*x2) as f32);
                        maxy_b.append_value(y1.max(*y2) as f32);
                        let wkb = minimal_wkb_linestring(*x1, *y1, *x2, *y2);
                        geom_b.append_value(&wkb);
                    }
                }
            }
        } else {
            let (ids, _) = self.build_graph_data();
            let generated_unit_count = ids.len();
            for (idx, &unit_id) in ids.iter().enumerate() {
                let (minx, miny, maxx, maxy) = self.generated_unit_bbox(idx, generated_unit_count);

                // Center of the bbox for the linestring
                let cx = ((minx + maxx) / 2.0) as f64;
                let cy = ((miny + maxy) / 2.0) as f64;

                id_b.append_value(unit_id);
                unit_id_b.append_value(unit_id);
                weight_b.append_value(100.0f32);
                stem_role_b.append_value("mainstem");
                minx_b.append_value(minx);
                miny_b.append_value(miny);
                maxx_b.append_value(maxx);
                maxy_b.append_value(maxy);

                let wkb = minimal_wkb_linestring(cx - 0.1, cy, cx + 0.1, cy);
                geom_b.append_value(&wkb);
            }
        }

        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(id_b.finish()),
                Arc::new(unit_id_b.finish()),
                Arc::new(weight_b.finish()),
                Arc::new(stem_role_b.finish()),
                Arc::new(minx_b.finish()),
                Arc::new(miny_b.finish()),
                Arc::new(maxx_b.finish()),
                Arc::new(maxy_b.finish()),
                Arc::new(geom_b.finish()),
            ],
        )
        .unwrap();

        writer.write(&batch).unwrap();
        writer.close().unwrap();
    }

    fn write_raster_stubs(&self, root: &Path) {
        std::fs::write(root.join("flow_dir.tif"), b"stub").unwrap();
        std::fs::write(root.join("flow_acc.tif"), b"stub").unwrap();
    }

    // -----------------------------------------------------------------------
    // Internal data generation
    // -----------------------------------------------------------------------

    /// Build the (ids, upstream_ids) vectors for the graph.
    ///
    /// Linear chain: unit 1 is headwater, unit i has upstream=[i-1].
    /// DAG mode appends four extra units forming a diamond on top of the chain.
    fn build_graph_data(&self) -> (Vec<i64>, Vec<Vec<i64>>) {
        let n = self.unit_count;
        let mut ids: Vec<i64>;
        let mut upstream: Vec<Vec<i64>>;

        if let Some(customs) = &self.custom_catchments {
            // Build a linear chain from custom IDs: first is headwater.
            ids = customs.iter().map(|c| c.id).collect();
            upstream = Vec::with_capacity(ids.len());
            for (idx, _) in ids.iter().enumerate() {
                if idx == 0 {
                    upstream.push(vec![]);
                } else {
                    upstream.push(vec![ids[idx - 1]]);
                }
            }
        } else {
            ids = (1..=(n as i64)).collect();
            upstream = Vec::with_capacity(n);

            // Unit 1 is a headwater; unit i has upstream = [i-1].
            for i in 1..=(n as i64) {
                if i == 1 {
                    upstream.push(vec![]);
                } else {
                    upstream.push(vec![i - 1]);
                }
            }
        }

        if self.dag_diamond {
            // Diamond: N+1 and N+2 are headwaters; N+3 merges both;
            // N+4 is downstream of N+3 and N (existing chain outlet).
            let base = n as i64;
            let hw1 = base + 1;
            let hw2 = base + 2;
            let merge = base + 3;
            let outlet = base + 4;

            ids.push(hw1);
            upstream.push(vec![]);

            ids.push(hw2);
            upstream.push(vec![]);

            ids.push(merge);
            upstream.push(vec![hw1, hw2]);

            ids.push(outlet);
            upstream.push(vec![base, merge]);
        }

        (ids, upstream)
    }

    fn generated_unit_count(&self) -> usize {
        if self.dag_diamond {
            self.unit_count + 4
        } else if self.multilevel_nested {
            nested_fixture_units().len()
        } else {
            self.unit_count
        }
    }

    fn generated_unit_bbox(&self, idx: usize, unit_count: usize) -> (f32, f32, f32, f32) {
        let miny = 0.0f32;
        let maxy = 0.4f32;

        if let Some((min_lon, max_lon)) = self.generated_longitude_span {
            let slot_width = (max_lon - min_lon) / unit_count as f64;
            let minx = min_lon + idx as f64 * slot_width;
            let maxx = minx + slot_width * 0.8;
            return (minx as f32, miny, maxx as f32, maxy);
        }

        let i = idx + 1; // 1-based for legacy bbox spacing.
        let minx = (i as f32) * 0.5;
        let maxx = (i as f32) * 0.5 + 0.4;
        (minx, miny, maxx, maxy)
    }

    fn fixture_units(&self) -> Option<Vec<FixtureUnit>> {
        self.multilevel_nested.then(nested_fixture_units)
    }

    fn fixture_graph_rows(&self) -> Option<Vec<FixtureGraphRow>> {
        self.multilevel_nested.then(nested_fixture_graph_rows)
    }
}

fn nested_fixture_units() -> Vec<FixtureUnit> {
    vec![
        FixtureUnit::new(1, 0, None, 30.0, Some(60.0), (0.0, -2.0, 4.0, 0.0)),
        FixtureUnit::new(10, 1, Some(1), 10.0, None, (0.0, -1.0, 1.0, 0.0)),
        FixtureUnit::new(20, 1, Some(1), 10.0, None, (1.0, -1.0, 2.0, 0.0)),
        FixtureUnit::new(30, 1, Some(1), 10.0, None, (2.0, -1.0, 3.0, 0.0)),
    ]
}

fn nested_fixture_graph_rows() -> Vec<FixtureGraphRow> {
    vec![
        FixtureGraphRow::new(1, 0, vec![]),
        FixtureGraphRow::new(10, 1, vec![]),
        FixtureGraphRow::new(20, 1, vec![10]),
        FixtureGraphRow::new(30, 1, vec![20]),
    ]
}

// ---------------------------------------------------------------------------
// WKB helpers
// ---------------------------------------------------------------------------

fn minimal_wkb_point(x: f64, y: f64) -> Vec<u8> {
    let mut wkb = Vec::new();
    wkb.push(1u8); // little-endian
    wkb.extend_from_slice(&1u32.to_le_bytes()); // wkbPoint = 1
    wkb.extend_from_slice(&x.to_le_bytes());
    wkb.extend_from_slice(&y.to_le_bytes());
    wkb
}

fn minimal_wkb_polygon(minx: f64, miny: f64, maxx: f64, maxy: f64) -> Vec<u8> {
    let mut wkb = Vec::new();
    wkb.push(1u8); // little-endian
    wkb.extend_from_slice(&3u32.to_le_bytes()); // polygon type
    wkb.extend_from_slice(&1u32.to_le_bytes()); // 1 ring
    wkb.extend_from_slice(&5u32.to_le_bytes()); // 5 points (closed)
    for (x, y) in [
        (minx, miny),
        (maxx, miny),
        (maxx, maxy),
        (minx, maxy),
        (minx, miny),
    ] {
        wkb.extend_from_slice(&x.to_le_bytes());
        wkb.extend_from_slice(&y.to_le_bytes());
    }
    wkb
}

fn generated_wkb_polygon(
    minx: f64,
    miny: f64,
    maxx: f64,
    maxy: f64,
    coords_per_ring: usize,
) -> Vec<u8> {
    if coords_per_ring <= 5 {
        return minimal_wkb_polygon(minx, miny, maxx, maxy);
    }

    regular_ngon_wkb_polygon(minx, miny, maxx, maxy, coords_per_ring)
}

fn regular_ngon_wkb_polygon(
    minx: f64,
    miny: f64,
    maxx: f64,
    maxy: f64,
    coords_per_ring: usize,
) -> Vec<u8> {
    let side_count = coords_per_ring - 1;
    let cx = (minx + maxx) / 2.0;
    let cy = (miny + maxy) / 2.0;
    let rx = (maxx - minx) / 2.0;
    let ry = (maxy - miny) / 2.0;
    let mut points = Vec::with_capacity(coords_per_ring);

    for side in 0..side_count {
        let theta = (side as f64) * std::f64::consts::TAU / (side_count as f64);
        points.push((cx + rx * theta.cos(), cy + ry * theta.sin()));
    }
    points.push(points[0]);

    let mut wkb = Vec::new();
    wkb.push(1u8); // little-endian
    wkb.extend_from_slice(&3u32.to_le_bytes()); // polygon type
    wkb.extend_from_slice(&1u32.to_le_bytes()); // 1 ring
    wkb.extend_from_slice(&(coords_per_ring as u32).to_le_bytes());
    for (x, y) in points {
        wkb.extend_from_slice(&x.to_le_bytes());
        wkb.extend_from_slice(&y.to_le_bytes());
    }
    wkb
}

fn minimal_wkb_linestring(x1: f64, y1: f64, x2: f64, y2: f64) -> Vec<u8> {
    let mut wkb = Vec::new();
    wkb.push(1u8); // little-endian
    wkb.extend_from_slice(&2u32.to_le_bytes()); // linestring type
    wkb.extend_from_slice(&2u32.to_le_bytes()); // 2 points
    for (x, y) in [(x1, y1), (x2, y2)] {
        wkb.extend_from_slice(&x.to_le_bytes());
        wkb.extend_from_slice(&y.to_le_bytes());
    }
    wkb
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::DatasetSession;

    #[test]
    fn test_minimal_dataset_opens() {
        let (_dir, root) = DatasetBuilder::new(3).build();
        let session = DatasetSession::open_path(&root).expect("minimal dataset should open");
        assert_eq!(session.manifest().unit_count().get(), 3);
        assert!(session.snap().is_none());
        assert!(session.raster_paths().is_none());
    }

    #[test]
    fn test_dataset_with_snap_opens() {
        let (_dir, root) = DatasetBuilder::new(5).with_snap().build();
        let session = DatasetSession::open_path(&root).expect("dataset with snap should open");
        assert!(session.snap().is_some());
    }

    #[test]
    fn test_dataset_with_rasters_opens() {
        let (_dir, root) = DatasetBuilder::new(4).with_rasters().build();
        let session = DatasetSession::open_path(&root).expect("dataset with rasters should open");
        assert!(session.raster_paths().is_some());
        let rp = session.raster_paths().unwrap();
        assert!(rp.flow_dir().exists());
        assert!(rp.flow_acc().exists());
    }

    #[test]
    fn test_dataset_with_small_row_groups() {
        let (_dir, root) = DatasetBuilder::new(10).with_row_group_size(3).build();
        let session =
            DatasetSession::open_path(&root).expect("small row group dataset should open");
        assert_eq!(session.manifest().unit_count().get(), 10);
    }

    #[test]
    fn test_dag_dataset_opens() {
        let (_dir, root) = DatasetBuilder::new(3).with_dag().build();
        let session = DatasetSession::open_path(&root).expect("dag dataset should open");
        // DAG mode adds 4 extra units
        assert_eq!(session.manifest().unit_count().get(), 7);
        assert_eq!(session.topology(), hfx_core::Topology::Dag);
    }

    #[test]
    fn test_multilevel_nested_dataset_opens() {
        let (_dir, root) = DatasetBuilder::new(1).with_multilevel_nested().build();
        let session =
            DatasetSession::open_path(&root).expect("multi-level nested dataset should open");
        assert_eq!(session.manifest().unit_count().get(), 4);
        assert_eq!(session.topology(), hfx_core::Topology::Tree);
        assert_eq!(session.graph().len(), 4);
    }

    #[test]
    fn test_graph_has_correct_row_count() {
        let (_dir, root) = DatasetBuilder::new(5).build();
        let session = DatasetSession::open_path(&root).unwrap();
        assert_eq!(session.graph().len(), 5);
    }

    #[test]
    fn test_catchments_have_correct_count() {
        let (_dir, root) = DatasetBuilder::new(4).build();
        let session = DatasetSession::open_path(&root).unwrap();
        assert_eq!(session.catchments().total_rows(), 4);
    }

    #[test]
    fn test_dataset_with_complex_generated_polygons_opens() {
        let (_dir, root) = DatasetBuilder::new(6).with_polygon_complexity(12).build();
        let session =
            DatasetSession::open_path(&root).expect("complex polygon dataset should open");
        assert_eq!(session.manifest().unit_count().get(), 6);
        assert_eq!(session.catchments().total_rows(), 6);
    }

    #[test]
    fn test_large_generated_fixture_fits_longitude_span() {
        let builder = DatasetBuilder::new(2_500)
            .with_longitude_span(-179.0, 179.0)
            .with_polygon_complexity(1_500);
        let terminal = builder
            .generated_terminal_unit_center()
            .expect("generated fixture should have a terminal unit");
        assert!((-180.0..=180.0).contains(&terminal.lon));
        assert!((-90.0..=90.0).contains(&terminal.lat));

        let (_dir, root) = builder.build();
        let session =
            DatasetSession::open_path(&root).expect("large generated dataset should open");
        assert_eq!(session.manifest().unit_count().get(), 2_500);
        assert_eq!(session.catchments().total_rows(), 2_500);
    }

    #[test]
    fn test_full_dataset_opens() {
        let (_dir, root) = DatasetBuilder::new(6)
            .with_snap()
            .with_rasters()
            .with_row_group_size(2)
            .build();
        let session = DatasetSession::open_path(&root).expect("full dataset should open");
        assert_eq!(session.manifest().unit_count().get(), 6);
        assert!(session.snap().is_some());
        assert!(session.raster_paths().is_some());
    }

    #[test]
    fn test_custom_catchments_dataset_opens() {
        let catchments = vec![
            TestCatchment {
                id: 10,
                area_km2: 5.0,
                up_area_km2: Some(100.0),
                polygon: (1.0, 0.0, 1.4, 0.4),
            },
            TestCatchment {
                id: 20,
                area_km2: 8.0,
                up_area_km2: None,
                polygon: (1.5, 0.0, 1.9, 0.4),
            },
        ];
        let (_dir, root) = DatasetBuilder::new(2)
            .with_custom_catchments(catchments)
            .build();
        let session =
            DatasetSession::open_path(&root).expect("custom catchments dataset should open");
        assert_eq!(session.manifest().unit_count().get(), 2);
    }

    #[test]
    fn test_custom_snap_targets_dataset_opens() {
        let catchments = vec![
            TestCatchment {
                id: 1,
                area_km2: 10.0,
                up_area_km2: None,
                polygon: (0.5, 0.0, 0.9, 0.4),
            },
            TestCatchment {
                id: 2,
                area_km2: 10.0,
                up_area_km2: None,
                polygon: (1.0, 0.0, 1.4, 0.4),
            },
        ];
        let targets = vec![
            TestSnapTarget {
                id: 1,
                catchment_id: 1,
                weight: 50.0,
                is_mainstem: true,
                geometry: TestSnapGeometry::Point(0.7, 0.2),
            },
            TestSnapTarget {
                id: 2,
                catchment_id: 2,
                weight: 100.0,
                is_mainstem: false,
                geometry: TestSnapGeometry::LineString(1.1, 0.2, 1.3, 0.2),
            },
        ];
        let (_dir, root) = DatasetBuilder::new(2)
            .with_custom_catchments(catchments)
            .with_custom_snap_targets(targets)
            .build();
        let session =
            DatasetSession::open_path(&root).expect("custom snap targets dataset should open");
        assert!(session.snap().is_some());
    }

    #[test]
    fn test_generated_polygon_complexity_controls_ring_point_count() {
        let wkb = generated_wkb_polygon(0.0, 0.0, 2.0, 2.0, 1_500);
        assert_eq!(wkb[0], 1);
        assert_eq!(u32_at(&wkb, 1), 3);
        assert_eq!(u32_at(&wkb, 5), 1);
        assert_eq!(u32_at(&wkb, 9), 1_500);

        let first = point_at(&wkb, 13);
        let last = point_at(&wkb, 13 + (1_499 * 16));
        assert_eq!(first, last);
    }

    #[test]
    fn test_polygon_complexity_keeps_rectangle_until_above_default_ring_size() {
        let wkb = generated_wkb_polygon(0.0, 0.0, 2.0, 2.0, 5);
        assert_eq!(u32_at(&wkb, 9), 5);
        assert_eq!(point_at(&wkb, 13), (0.0, 0.0));
        assert_eq!(point_at(&wkb, 13 + (4 * 16)), (0.0, 0.0));
    }

    fn u32_at(wkb: &[u8], offset: usize) -> u32 {
        u32::from_le_bytes(wkb[offset..offset + 4].try_into().unwrap())
    }

    fn point_at(wkb: &[u8], offset: usize) -> (f64, f64) {
        let x = f64::from_le_bytes(wkb[offset..offset + 8].try_into().unwrap());
        let y = f64::from_le_bytes(wkb[offset + 8..offset + 16].try_into().unwrap());
        (x, y)
    }
}
