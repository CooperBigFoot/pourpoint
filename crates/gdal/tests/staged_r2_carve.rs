//! staged_r2_carve : PublicHfxRoot × BoundedSeamSearch → WitnessedRequiredD8Carve
//! Manual command: `POURPOINT_STAGED_R2_CARVE=1 cargo test -p pourpoint-gdal --test staged_r2_carve -- --ignored --nocapture`

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::fs::File;
use std::future::Future;
use std::ops::Range;
use std::path::Path;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use futures_util::StreamExt;
use futures_util::stream::{self, BoxStream};
use gdal::Dataset as GdalDataset;
use geo::{BoundingRect, Coord, Rect};
use hfx::{BoundingBox, CatchmentUnit, UnitId};
use object_store::path::Path as ObjectPath;
use object_store::{
    Attributes, CopyOptions, GetOptions, GetResult, GetResultPayload, ListResult, MultipartUpload,
    ObjectMeta, ObjectStore, ObjectStoreExt, PutMultipartOptions, PutOptions, PutPayload,
    PutResult, RenameOptions, Result as StoreResult,
};
use pourpoint_core::algo::{
    Crs, GeoCoord, NativeCoord, RasterSource, decode_wkb_multi_polygon, forward,
    geodesic_area_multi, inverse,
};
use pourpoint_core::session::{DatasetSession, RasterKind};
use pourpoint_core::source::DatasetSource;
use pourpoint_core::{
    CrossedTileAxes, D8RasterHandle, DelineationOptions, Engine, LevelResolvedOutlet,
    LevelSelection, RasterWindowCoverage, RefinementMode, ResolutionMethod, ResolverConfig,
    SnapStrategy, TerminalRefinement,
};
use pourpoint_gdal::GdalRasterSource;
use serde::Serialize;
use serde_json::{Value, json};
use tiff::decoder::{Decoder, DecodingResult};
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::{Layer, Registry};

const PUBLIC_ROOT: &str = "https://basin-delineations-public.upstream.tech/grit/hfx-v0.3.0/";
const MANIFEST_KEY: &str = "manifest.json";
const FLOW_DIR_KEY: &str = "aux/d8/flow_dir.tif";
const FLOW_ACC_KEY: &str = "aux/d8/flow_acc.tif";
const INDEX_END: u64 = 24_507_158;

const MAX_PLANNED_TILE_COUNT: u64 = 65_536;
const MAX_COMPRESSED_CHUNK_BYTES: u64 = 16_777_216;
const MAX_COVERED_CHUNK_BYTES: u64 = 1_073_741_824;
const MAX_DECODED_CHUNK_BYTES: u64 = 8_388_608;
const MAX_WINDOW_ALLOCATION_BYTES: u64 = 1_073_741_824;

const CANDIDATE_BUDGET: usize = 128;
const TILE_SIZE: u32 = 512;
const BAND_HALF_WIDTH_PIXELS: f64 = 32.0;
const BAND_HALF_LENGTH_PIXELS: f64 = 4096.0;
const INNER_SUBWINDOW_PIXELS: u32 = 480;
const INNER_TILE_INSET_PIXELS: u32 = 16;
const WIDE_HALF_SPAN_PIXELS: u32 = 496;

#[derive(Debug)]
struct EnvGuards {
    previous: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

impl EnvGuards {
    fn install(cache: &Path) -> Self {
        let previous = [
            "HFX_CACHE_DIR",
            "AWS_ACCESS_KEY_ID",
            "AWS_SECRET_ACCESS_KEY",
        ]
        .into_iter()
        .map(|name| (name, std::env::var_os(name)))
        .collect();
        // SAFETY: this integration-test binary contains exactly one test, performs
        // every mutation before starting work, retains this guard, and restores
        // every prior value before exit.
        unsafe {
            std::env::set_var("HFX_CACHE_DIR", cache);
            std::env::set_var("AWS_ACCESS_KEY_ID", "deliberately-unusable-access-key");
            std::env::set_var(
                "AWS_SECRET_ACCESS_KEY",
                "deliberately-unusable-secret-access-key",
            );
        }
        Self { previous }
    }
}

impl Drop for EnvGuards {
    fn drop(&mut self) {
        // SAFETY: this integration-test binary contains exactly one test; its
        // guards remain alive through all work and restore every prior value
        // before the binary exits.
        unsafe {
            for (name, value) in &self.previous {
                match value {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
struct PathObservation {
    get_opts_calls: u64,
    head_calls: u64,
    full_get_calls: u64,
    ranged_get_calls: u64,
    get_opts_ranges: Vec<Range<u64>>,
    get_ranges_calls: u64,
    get_ranges_ranges: Vec<Range<u64>>,
}

impl PathObservation {
    fn subtract(&self, earlier: &Self) -> Self {
        Self {
            get_opts_calls: self.get_opts_calls - earlier.get_opts_calls,
            head_calls: self.head_calls - earlier.head_calls,
            full_get_calls: self.full_get_calls - earlier.full_get_calls,
            ranged_get_calls: self.ranged_get_calls - earlier.ranged_get_calls,
            get_opts_ranges: self.get_opts_ranges[earlier.get_opts_ranges.len()..].to_vec(),
            get_ranges_calls: self.get_ranges_calls - earlier.get_ranges_calls,
            get_ranges_ranges: self.get_ranges_ranges[earlier.get_ranges_ranges.len()..].to_vec(),
        }
    }

    fn evidence(&self) -> StoreRasterEvidence {
        StoreRasterEvidence {
            get_opts_calls: self.get_opts_calls,
            head_calls: self.head_calls,
            full_get_calls: self.full_get_calls,
            ranged_get_calls: self.ranged_get_calls,
            get_opts_range_bytes: range_bytes(&self.get_opts_ranges),
            get_ranges_calls: self.get_ranges_calls,
            get_ranges_range_count: self.get_ranges_ranges.len() as u64,
            get_ranges_range_bytes: range_bytes(&self.get_ranges_ranges),
            max_get_ranges_range_bytes: self
                .get_ranges_ranges
                .iter()
                .map(|range| range.end - range.start)
                .max()
                .unwrap_or(0),
            payload_ranges_beyond_24507158: self
                .get_ranges_ranges
                .iter()
                .filter(|range| range.end > INDEX_END)
                .count() as u64,
        }
    }
}

fn range_bytes(ranges: &[Range<u64>]) -> u64 {
    ranges.iter().map(|range| range.end - range.start).sum()
}

#[derive(Debug, Clone, Default)]
struct StoreSnapshot {
    paths: BTreeMap<String, PathObservation>,
}

impl StoreSnapshot {
    fn path(&self, key: &str) -> PathObservation {
        self.paths.get(key).cloned().unwrap_or_default()
    }
}

#[derive(Debug, Default)]
struct StoreState {
    paths: BTreeMap<String, PathObservation>,
    operation_calls: u64,
}

#[derive(Debug)]
struct SyntheticManifestStore {
    inner: Arc<dyn ObjectStore>,
    manifest_path: ObjectPath,
    manifest_bytes: Bytes,
    manifest_meta: ObjectMeta,
    state: Mutex<StoreState>,
    mutation_attempts: AtomicU64,
}

impl SyntheticManifestStore {
    fn snapshot(&self) -> StoreSnapshot {
        StoreSnapshot {
            paths: self
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .paths
                .clone(),
        }
    }

    fn record_operation(&self) {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .operation_calls += 1;
    }

    fn record_get_opts(&self, location: &ObjectPath, options: &GetOptions) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.operation_calls += 1;
        let observation = state.paths.entry(location.to_string()).or_default();
        observation.get_opts_calls += 1;
        if options.head {
            observation.head_calls += 1;
        } else if let Some(range) = &options.range {
            observation.ranged_get_calls += 1;
            if let Ok(range) = range.as_range(if location == &self.manifest_path {
                self.manifest_bytes.len() as u64
            } else {
                u64::MAX
            }) {
                observation.get_opts_ranges.push(range);
            }
        } else {
            observation.full_get_calls += 1;
        }
    }

    fn record_get_ranges(&self, location: &ObjectPath, ranges: &[Range<u64>]) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.operation_calls += 1;
        let observation = state.paths.entry(location.to_string()).or_default();
        observation.get_ranges_calls += 1;
        observation.get_ranges_ranges.extend_from_slice(ranges);
    }

    fn reject_mutation<T>(&self, operation: &'static str) -> StoreResult<T> {
        self.record_operation();
        self.mutation_attempts.fetch_add(1, Ordering::SeqCst);
        Err(object_store::Error::NotSupported {
            source: format!("read-only synthetic-manifest decorator rejects {operation}").into(),
        })
    }

    fn synthetic_result(
        &self,
        location: &ObjectPath,
        options: &GetOptions,
    ) -> StoreResult<GetResult> {
        if options.if_match.is_some()
            || options.if_none_match.is_some()
            || options.if_modified_since.is_some()
            || options.if_unmodified_since.is_some()
            || options.version.is_some()
        {
            return Err(object_store::Error::NotSupported {
                source: "synthetic manifest rejects conditional and version options".into(),
            });
        }
        let len = self.manifest_bytes.len() as u64;
        let range = if options.head {
            0..0
        } else {
            match &options.range {
                Some(range) => {
                    range
                        .as_range(len)
                        .map_err(|source| object_store::Error::Generic {
                            store: "SyntheticManifestStore",
                            source: Box::new(source),
                        })?
                }
                None => 0..len,
            }
        };
        let payload = if options.head {
            Bytes::new()
        } else {
            self.manifest_bytes
                .slice(range.start as usize..range.end as usize)
        };
        let mut meta = self.manifest_meta.clone();
        meta.location = location.clone();
        meta.size = len;
        Ok(GetResult {
            payload: GetResultPayload::Stream(stream::once(async move { Ok(payload) }).boxed()),
            meta,
            range,
            attributes: Attributes::default(),
        })
    }
}

impl fmt::Display for SyntheticManifestStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "SyntheticManifestStore(read-only over {})",
            self.inner
        )
    }
}

impl ObjectStore for SyntheticManifestStore {
    fn put_opts<'a, 'b, 'c>(
        &'a self,
        _location: &'b ObjectPath,
        _payload: PutPayload,
        _opts: PutOptions,
    ) -> Pin<Box<dyn Future<Output = StoreResult<PutResult>> + Send + 'c>>
    where
        'a: 'c,
        'b: 'c,
        Self: 'c,
    {
        Box::pin(async move { self.reject_mutation("put_opts") })
    }

    fn put_multipart_opts<'a, 'b, 'c>(
        &'a self,
        _location: &'b ObjectPath,
        _opts: PutMultipartOptions,
    ) -> Pin<Box<dyn Future<Output = StoreResult<Box<dyn MultipartUpload>>> + Send + 'c>>
    where
        'a: 'c,
        'b: 'c,
        Self: 'c,
    {
        Box::pin(async move { self.reject_mutation("put_multipart_opts") })
    }

    fn get_opts<'a, 'b, 'c>(
        &'a self,
        location: &'b ObjectPath,
        options: GetOptions,
    ) -> Pin<Box<dyn Future<Output = StoreResult<GetResult>> + Send + 'c>>
    where
        'a: 'c,
        'b: 'c,
        Self: 'c,
    {
        Box::pin(async move {
            self.record_get_opts(location, &options);
            if location == &self.manifest_path {
                self.synthetic_result(location, &options)
            } else {
                self.inner.get_opts(location, options).await
            }
        })
    }

    fn get_ranges<'a, 'b, 'c, 'd>(
        &'a self,
        location: &'b ObjectPath,
        ranges: &'c [Range<u64>],
    ) -> Pin<Box<dyn Future<Output = StoreResult<Vec<Bytes>>> + Send + 'd>>
    where
        'a: 'd,
        'b: 'd,
        'c: 'd,
        Self: 'd,
    {
        Box::pin(async move {
            self.record_get_ranges(location, ranges);
            if location == &self.manifest_path {
                ranges
                    .iter()
                    .map(|range| {
                        let options = GetOptions {
                            range: Some(object_store::GetRange::Bounded(range.clone())),
                            ..Default::default()
                        };
                        self.synthetic_result(location, &options)
                    })
                    .map(|result| async { result?.bytes().await })
                    .collect::<futures_util::stream::FuturesOrdered<_>>()
                    .collect::<Vec<_>>()
                    .await
                    .into_iter()
                    .collect()
            } else {
                self.inner.get_ranges(location, ranges).await
            }
        })
    }

    fn delete_stream(
        &self,
        _locations: BoxStream<'static, StoreResult<ObjectPath>>,
    ) -> BoxStream<'static, StoreResult<ObjectPath>> {
        self.record_operation();
        self.mutation_attempts.fetch_add(1, Ordering::SeqCst);
        stream::once(async {
            Err(object_store::Error::NotSupported {
                source: "read-only synthetic-manifest decorator rejects delete_stream".into(),
            })
        })
        .boxed()
    }

    fn list(&self, prefix: Option<&ObjectPath>) -> BoxStream<'static, StoreResult<ObjectMeta>> {
        self.record_operation();
        self.inner.list(prefix)
    }

    fn list_with_offset(
        &self,
        prefix: Option<&ObjectPath>,
        offset: &ObjectPath,
    ) -> BoxStream<'static, StoreResult<ObjectMeta>> {
        self.record_operation();
        self.inner.list_with_offset(prefix, offset)
    }

    fn list_with_delimiter<'a, 'b, 'c>(
        &'a self,
        prefix: Option<&'b ObjectPath>,
    ) -> Pin<Box<dyn Future<Output = StoreResult<ListResult>> + Send + 'c>>
    where
        'a: 'c,
        'b: 'c,
        Self: 'c,
    {
        self.record_operation();
        Box::pin(async move { self.inner.list_with_delimiter(prefix).await })
    }

    fn copy_opts<'a, 'b, 'c, 'd>(
        &'a self,
        _from: &'b ObjectPath,
        _to: &'c ObjectPath,
        _options: CopyOptions,
    ) -> Pin<Box<dyn Future<Output = StoreResult<()>> + Send + 'd>>
    where
        'a: 'd,
        'b: 'd,
        'c: 'd,
        Self: 'd,
    {
        Box::pin(async move { self.reject_mutation("copy_opts") })
    }

    fn rename_opts<'a, 'b, 'c, 'd>(
        &'a self,
        _from: &'b ObjectPath,
        _to: &'c ObjectPath,
        _options: RenameOptions,
    ) -> Pin<Box<dyn Future<Output = StoreResult<()>> + Send + 'd>>
    where
        'a: 'd,
        'b: 'd,
        'c: 'd,
        Self: 'd,
    {
        Box::pin(async move { self.reject_mutation("rename_opts") })
    }
}

#[derive(Debug, Clone, Default)]
struct RasterTelemetry {
    header_bytes: u64,
    tile_bytes: u64,
    tile_count: u64,
    window_pixels: u64,
}

#[derive(Debug, Clone, Default)]
struct TraceState {
    event_count: u64,
    declaration_index: Option<u64>,
    flow_dir: Option<RasterTelemetry>,
    flow_acc: Option<RasterTelemetry>,
    span_stages: HashMap<u64, String>,
    span_paths: HashMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Default)]
struct CaptureLayer {
    state: Arc<Mutex<TraceState>>,
}

#[derive(Default)]
struct TraceVisitor {
    strings: HashMap<String, String>,
    numbers: HashMap<String, u64>,
}

impl Visit for TraceVisitor {
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.numbers.insert(field.name().to_string(), value);
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.strings
            .insert(field.name().to_string(), value.to_string());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        let rendered = format!("{value:?}");
        self.strings.insert(
            field.name().to_string(),
            rendered
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
                .unwrap_or(&rendered)
                .to_string(),
        );
    }
}

impl<S> Layer<S> for CaptureLayer
where
    S: Subscriber,
{
    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        id: &tracing::span::Id,
        _context: Context<'_, S>,
    ) {
        if attrs.metadata().name() != "stage" {
            return;
        }
        let mut visitor = TraceVisitor::default();
        attrs.record(&mut visitor);
        if let Some(stage) = visitor.strings.get("stage")
            && matches!(
                stage.as_str(),
                "raster_localize_flow_dir" | "raster_localize_flow_acc"
            )
        {
            self.state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .span_stages
                .insert(id.into_u64(), stage.clone());
        }
    }

    fn on_record(
        &self,
        id: &tracing::span::Id,
        values: &tracing::span::Record<'_>,
        _context: Context<'_, S>,
    ) {
        let mut visitor = TraceVisitor::default();
        values.record(&mut visitor);
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if let (Some(stage), Some(path)) = (
            state.span_stages.get(&id.into_u64()).cloned(),
            visitor.strings.get("path"),
        ) {
            state
                .span_paths
                .entry(stage)
                .or_default()
                .push(path.clone());
        }
    }

    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let mut visitor = TraceVisitor::default();
        event.record(&mut visitor);
        if visitor.strings.get("message").map(String::as_str)
            != Some("localized selected D8 raster windows for refinement")
        {
            return;
        }
        let required = |name: &str| {
            *visitor
                .numbers
                .get(name)
                .unwrap_or_else(|| panic!("refinement event must contain {name}"))
        };
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.event_count += 1;
        state.flow_dir = Some(RasterTelemetry {
            header_bytes: required("flow_dir_cog_header_bytes"),
            tile_bytes: required("flow_dir_cog_tile_bytes"),
            tile_count: required("flow_dir_cog_tile_count"),
            window_pixels: required("flow_dir_window_pixels"),
        });
        state.flow_acc = Some(RasterTelemetry {
            header_bytes: required("flow_acc_cog_header_bytes"),
            tile_bytes: required("flow_acc_cog_tile_bytes"),
            tile_count: required("flow_acc_cog_tile_count"),
            window_pixels: required("flow_acc_window_pixels"),
        });
        state.declaration_index = Some(required("declaration_index"));
    }
}

fn object_path(root: &ObjectPath, artifact: &str) -> ObjectPath {
    ObjectPath::from(format!(
        "{}/{artifact}",
        root.as_ref().trim_end_matches('/')
    ))
}

fn d8_count(value: &Value) -> usize {
    value
        .get("auxiliary")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("live manifest root must contain an auxiliary array"))
        .iter()
        .filter(|entry| entry.get("schema").and_then(Value::as_str) == Some("hfx.aux.d8_raster.v2"))
        .count()
}

#[derive(Serialize)]
struct Evidence {
    input_coord: [f64; 2],
    resolved_coord: [f64; 2],
    resolved_terminal_id: i64,
    snap: SnapEvidence,
    upstream_count: usize,
    refinement: &'static str,
    route: RouteEvidence,
    areas_km2: AreaEvidence,
    seam_search: SeamSearchEvidence,
    terminal_windows: TerminalWindowsEvidence,
    overlap: OverlapEvidence,
    store: StoreEvidence,
    telemetry: TelemetryEvidence,
    ceilings: CeilingsEvidence,
    decoded: DecodedEvidence,
    live_manifest: LiveManifestEvidence,
    mutation_attempt_count: u64,
}

#[derive(Serialize)]
struct SnapEvidence {
    method: &'static str,
    strategy: &'static str,
    snap_id: i64,
    weight: f32,
    mainstem_status: Option<String>,
    distance_m: f64,
    candidates_considered: usize,
    declaration_name: String,
    declaration_artifact: String,
    references_levels: Vec<i16>,
    weight_semantics: String,
    declaration_status: &'static str,
    bounds_status: &'static str,
}

#[derive(Serialize)]
struct RouteEvidence {
    public_custom_domain: &'static str,
    object_store_builder: &'static str,
    skip_signature: bool,
    bogus_aws_credentials_installed: bool,
    ambient_aws_credentials_consulted: bool,
}

#[derive(Serialize)]
struct AreaEvidence {
    unrefined_terminal_geodesic: f64,
    refined_terminal_geodesic: f64,
    resolved_terminal_hfx_local: f32,
    status: &'static str,
}

#[derive(Serialize)]
struct StoreEvidence {
    seam_search: DeltaStoreEvidence,
    initial_carve: InitialStoreEvidence,
    retained_session_delta: DeltaStoreEvidence,
    observation_unit: &'static str,
}

#[derive(Serialize)]
struct InitialStoreEvidence {
    flow_dir: KeyedStoreRasterEvidence,
    flow_acc: KeyedStoreRasterEvidence,
}

#[derive(Serialize)]
struct DeltaStoreEvidence {
    flow_dir: StoreRasterEvidence,
    flow_acc: StoreRasterEvidence,
}

#[derive(Serialize)]
struct KeyedStoreRasterEvidence {
    key: &'static str,
    #[serde(flatten)]
    calls: StoreRasterEvidence,
}

#[derive(Debug, Clone, Serialize)]
struct StoreRasterEvidence {
    get_opts_calls: u64,
    head_calls: u64,
    full_get_calls: u64,
    ranged_get_calls: u64,
    get_opts_range_bytes: u64,
    get_ranges_calls: u64,
    get_ranges_range_count: u64,
    get_ranges_range_bytes: u64,
    max_get_ranges_range_bytes: u64,
    payload_ranges_beyond_24507158: u64,
}

#[derive(Serialize)]
struct TelemetryEvidence {
    event_count: u64,
    flow_dir: RasterTelemetryEvidence,
    flow_acc: RasterTelemetryEvidence,
}

#[derive(Serialize)]
struct RasterTelemetryEvidence {
    header_bytes: u64,
    tile_bytes: u64,
    tile_count: u64,
    window_pixels: u64,
    internal_path: String,
    direct_cached_path: String,
}

#[derive(Serialize)]
struct CeilingsEvidence {
    status: &'static str,
    flow_dir: RasterCeilingsEvidence,
    flow_acc: RasterCeilingsEvidence,
    f32_decoded_chunk_statement: &'static str,
}

#[allow(non_snake_case)]
#[derive(Serialize)]
struct RasterCeilingsEvidence {
    MAX_PLANNED_TILE_COUNT: CeilingEvidence,
    MAX_COMPRESSED_CHUNK_BYTES: CeilingEvidence,
    MAX_COVERED_CHUNK_BYTES: CeilingEvidence,
    MAX_DECODED_CHUNK_BYTES: CeilingEvidence,
    MAX_WINDOW_ALLOCATION_BYTES: CeilingEvidence,
}

#[derive(Serialize)]
struct CeilingEvidence {
    observed: u64,
    ceiling: u64,
    margin: u64,
}

#[derive(Serialize)]
struct DecodedEvidence {
    flow_dir: FlowDirDecodedEvidence,
    flow_acc: FlowAccDecodedEvidence,
    claim: &'static str,
}

#[derive(Serialize)]
struct FlowDirDecodedEvidence {
    sample_type: &'static str,
    width: u32,
    height: u32,
    distinct_values: Vec<u8>,
    nodata_255_count: u64,
    nodata_255_fraction: f64,
    legal_grass_non_nodata_count: u64,
    legal_grass_non_nodata_fraction: f64,
    distinct_cap: u64,
    distinct_cap_headroom_over_legal_plus_nodata: u64,
    minimum_legal_fraction: f64,
}

#[derive(Serialize)]
struct FlowAccDecodedEvidence {
    sample_type: &'static str,
    width: u32,
    height: u32,
    nan_count: u64,
    nan_fraction: f64,
    non_nan_count: u64,
    non_nan_fraction: f64,
    non_nan_min: f32,
    non_nan_max: f32,
    magnitude_ceiling_km2: f64,
    minimum_non_nan_fraction: f64,
}

#[derive(Serialize)]
struct LiveManifestEvidence {
    byte_equal: bool,
    d8_declaration_present: bool,
}

#[derive(Serialize)]
struct SeamSearchEvidence {
    candidate_budget: usize,
    candidates_tried: usize,
    band_half_width_pixels: u32,
    band_half_length_pixels: u32,
    flow_dir_x_seam_coordinates: Vec<f64>,
    flow_dir_y_seam_coordinates: Vec<f64>,
    flow_acc_x_seam_coordinates: Vec<f64>,
    flow_acc_y_seam_coordinates: Vec<f64>,
    selected_candidate_input_coord: [f64; 2],
    selected_resolved_terminal_id: i64,
}

#[derive(Serialize)]
struct TerminalWindowsEvidence {
    flow_dir: TerminalWindowEvidence,
    flow_acc: TerminalWindowEvidence,
}

#[derive(Serialize)]
struct TerminalWindowEvidence {
    tile_count: u64,
    covered_tile_indexes: Vec<u32>,
    covered_tile_col_rows: Vec<[u32; 2]>,
    crossed_axes: &'static str,
    witnessed_axes: Vec<&'static str>,
    unwitnessed_axes: Vec<&'static str>,
}

#[derive(Serialize)]
struct OverlapEvidence {
    inner_subwindow_width_pixels: u32,
    inner_subwindow_height_pixels: u32,
    inner_tile_inset_pixels: u32,
    minimum_safe_inset_pixels: u32,
    from_bbox_padding_pixels_per_side: u32,
    geotransform_origin_tolerance_pixels: f64,
    flow_dir: RasterOverlapEvidence,
    flow_acc: RasterOverlapEvidence,
}

#[derive(Serialize)]
struct RasterOverlapEvidence {
    wide_requested_width_pixels: u32,
    wide_requested_height_pixels: u32,
    wide_window_covered_tile_indexes: Vec<u32>,
    wide_window_crossed_axes: &'static str,
    inner_window_covered_tile_index: u32,
    shared_width_pixels: usize,
    shared_height_pixels: usize,
    real_sample_count: usize,
    agreement: &'static str,
}

#[derive(Debug, Clone, Copy)]
struct GridObservation {
    origin_x: f64,
    origin_y: f64,
    pixel_width: f64,
    pixel_height: f64,
    raster_width: u32,
    raster_height: u32,
    tile_width: u32,
    tile_height: u32,
}

impl GridObservation {
    fn from_coverage(name: &str, coverage: &RasterWindowCoverage) -> Self {
        assert_eq!(
            coverage.tile_width(),
            TILE_SIZE,
            "{name} tile width must be 512"
        );
        assert_eq!(
            coverage.tile_height(),
            TILE_SIZE,
            "{name} tile height must be 512"
        );
        assert!(
            coverage.pixel_width() > 0.0,
            "{name} pixel width must be positive"
        );
        assert!(
            coverage.pixel_height() < 0.0,
            "{name} pixel height must be negative"
        );
        assert!(
            coverage.raster_width() > 0 && coverage.raster_height() > 0,
            "{name} raster dimensions must be nonzero"
        );
        Self {
            origin_x: coverage.origin_x(),
            origin_y: coverage.origin_y(),
            pixel_width: coverage.pixel_width(),
            pixel_height: coverage.pixel_height(),
            raster_width: coverage.raster_width(),
            raster_height: coverage.raster_height(),
            tile_width: coverage.tile_width(),
            tile_height: coverage.tile_height(),
        }
    }

    fn x_bounds(self) -> (f64, f64) {
        sorted_pair(
            self.origin_x,
            self.origin_x + f64::from(self.raster_width) * self.pixel_width,
        )
    }

    fn y_bounds(self) -> (f64, f64) {
        sorted_pair(
            self.origin_y,
            self.origin_y + f64::from(self.raster_height) * self.pixel_height,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SeamAxis {
    X,
    Y,
}

#[derive(Debug, Clone, Copy)]
struct NativeBand {
    axis: SeamAxis,
    seam_coordinate: f64,
    pixel_size: f64,
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
}

impl NativeBand {
    fn contains(self, coordinate: NativeCoord) -> bool {
        (self.min_x..=self.max_x).contains(&coordinate.x())
            && (self.min_y..=self.max_y).contains(&coordinate.y())
    }

    fn perpendicular_distance_pixels(self, coordinate: NativeCoord) -> f64 {
        match self.axis {
            SeamAxis::X => (coordinate.x() - self.seam_coordinate).abs() / self.pixel_size,
            SeamAxis::Y => (coordinate.y() - self.seam_coordinate).abs() / self.pixel_size,
        }
    }
}

#[derive(Debug)]
struct OrderedCandidate {
    unit: CatchmentUnit,
    lies_in_x_band: bool,
    lies_in_y_band: bool,
    minimum_perpendicular_distance_pixels: f64,
}

#[derive(Debug, Clone, Copy)]
struct PredictedPixelWindow {
    col_off: u32,
    row_off: u32,
    width: u32,
    height: u32,
}

impl PredictedPixelWindow {
    fn col_end(self) -> u32 {
        self.col_off
            .checked_add(self.width)
            .expect("predicted column end must not overflow")
    }

    fn row_end(self) -> u32 {
        self.row_off
            .checked_add(self.height)
            .expect("predicted row end must not overflow")
    }

    fn covered_tile_col_rows(self, grid: GridObservation) -> Vec<(u32, u32)> {
        let first_col = self.col_off / grid.tile_width;
        let last_col = (self.col_end() - 1) / grid.tile_width;
        let first_row = self.row_off / grid.tile_height;
        let last_row = (self.row_end() - 1) / grid.tile_height;
        let mut tiles = Vec::new();
        for row in first_row..=last_row {
            for col in first_col..=last_col {
                tiles.push((col, row));
            }
        }
        tiles
    }

    fn crossed_axes(self, grid: GridObservation) -> (bool, bool) {
        let tiles = self.covered_tile_col_rows(grid);
        let columns = tiles
            .iter()
            .map(|(column, _)| *column)
            .collect::<BTreeSet<_>>();
        let rows = tiles.iter().map(|(_, row)| *row).collect::<BTreeSet<_>>();
        (columns.len() > 1, rows.len() > 1)
    }
}

enum OverlapAttempt {
    Witness(RasterOverlapEvidence),
    LocalizationFailure,
    DimensionPreconditionFailure,
    AllNodata,
}

fn sorted_pair(left: f64, right: f64) -> (f64, f64) {
    if left <= right {
        (left, right)
    } else {
        (right, left)
    }
}

fn crs_from_handle(handle: &D8RasterHandle) -> Crs {
    let raw = handle.crs().as_str();
    let code = raw
        .strip_prefix("EPSG:")
        .unwrap_or_else(|| panic!("D8 raster CRS must carry EPSG: prefix, got {raw}"))
        .parse::<u32>()
        .unwrap_or_else(|error| panic!("D8 raster CRS code must parse from {raw}: {error}"));
    Crs::try_from(code)
        .unwrap_or_else(|error| panic!("D8 raster CRS {raw} must be supported: {error}"))
}

fn nearest_internal_seams(
    grid: GridObservation,
    axis: SeamAxis,
    center: NativeCoord,
) -> Vec<(u32, f64)> {
    let (dimension, origin, pixel_size, center_ordinate, bounds) = match axis {
        SeamAxis::X => (
            grid.raster_width,
            grid.origin_x,
            grid.pixel_width,
            center.x(),
            grid.x_bounds(),
        ),
        SeamAxis::Y => (
            grid.raster_height,
            grid.origin_y,
            grid.pixel_height,
            center.y(),
            grid.y_bounds(),
        ),
    };
    let tile_count = dimension.div_ceil(TILE_SIZE);
    let mut seams = (1..tile_count)
        .filter_map(|tile_index| {
            let coordinate = origin + f64::from(tile_index * TILE_SIZE) * pixel_size;
            (coordinate > bounds.0 && coordinate < bounds.1).then_some((tile_index, coordinate))
        })
        .collect::<Vec<_>>();
    seams.sort_by(|left, right| {
        (left.1 - center_ordinate)
            .abs()
            .total_cmp(&(right.1 - center_ordinate).abs())
            .then_with(|| left.0.cmp(&right.0))
    });
    seams.truncate(4);
    seams
}

fn bands_for_seams(
    grid: GridObservation,
    axis: SeamAxis,
    center: NativeCoord,
    seams: &[(u32, f64)],
) -> Vec<NativeBand> {
    let x_bounds = grid.x_bounds();
    let y_bounds = grid.y_bounds();
    seams
        .iter()
        .map(|(_, seam)| {
            let (raw_min_x, raw_max_x, raw_min_y, raw_max_y, pixel_size) = match axis {
                SeamAxis::X => (
                    *seam - BAND_HALF_WIDTH_PIXELS * grid.pixel_width,
                    *seam + BAND_HALF_WIDTH_PIXELS * grid.pixel_width,
                    center.y() - BAND_HALF_LENGTH_PIXELS * grid.pixel_height.abs(),
                    center.y() + BAND_HALF_LENGTH_PIXELS * grid.pixel_height.abs(),
                    grid.pixel_width,
                ),
                SeamAxis::Y => (
                    center.x() - BAND_HALF_LENGTH_PIXELS * grid.pixel_width,
                    center.x() + BAND_HALF_LENGTH_PIXELS * grid.pixel_width,
                    *seam - BAND_HALF_WIDTH_PIXELS * grid.pixel_height.abs(),
                    *seam + BAND_HALF_WIDTH_PIXELS * grid.pixel_height.abs(),
                    grid.pixel_height.abs(),
                ),
            };
            let band = NativeBand {
                axis,
                seam_coordinate: *seam,
                pixel_size,
                min_x: raw_min_x.max(x_bounds.0),
                min_y: raw_min_y.max(y_bounds.0),
                max_x: raw_max_x.min(x_bounds.1),
                max_y: raw_max_y.min(y_bounds.1),
            };
            assert!(
                band.min_x < band.max_x && band.min_y < band.max_y,
                "clipped seam band must remain non-degenerate: {band:?}"
            );
            band
        })
        .collect()
}

fn geographic_query_bbox(crs: Crs, band: NativeBand) -> BoundingBox {
    let xs = [band.min_x, (band.min_x + band.max_x) / 2.0, band.max_x];
    let ys = [band.min_y, (band.min_y + band.max_y) / 2.0, band.max_y];
    let mut min_lon = f64::INFINITY;
    let mut min_lat = f64::INFINITY;
    let mut max_lon = f64::NEG_INFINITY;
    let mut max_lat = f64::NEG_INFINITY;
    for x in xs {
        for y in ys {
            let geographic = inverse(crs, NativeCoord::new(x, y)).unwrap_or_else(|error| {
                panic!("clipped native seam-band point ({x}, {y}) must inverse-project: {error}")
            });
            min_lon = min_lon.min(geographic.lon);
            min_lat = min_lat.min(geographic.lat);
            max_lon = max_lon.max(geographic.lon);
            max_lat = max_lat.max(geographic.lat);
        }
    }
    let min_lon = (min_lon - 0.001).max(-180.0) as f32;
    let min_lat = (min_lat - 0.001).max(-90.0) as f32;
    let max_lon = (max_lon + 0.001).min(180.0) as f32;
    let max_lat = (max_lat + 0.001).min(90.0) as f32;
    BoundingBox::new(min_lon, min_lat, max_lon, max_lat).unwrap_or_else(|error| {
        panic!(
            "expanded seam-band geographic envelope [{min_lon}, {min_lat}, {max_lon}, {max_lat}] must be valid: {error}"
        )
    })
}

fn discover_candidates(
    session: &DatasetSession,
    selected_level: hfx::Level,
    crs: Crs,
    bands: &[NativeBand],
) -> Vec<OrderedCandidate> {
    let mut units = BTreeMap::<UnitId, CatchmentUnit>::new();
    for band in bands {
        let query_bbox = geographic_query_bbox(crs, *band);
        for unit in session
            .catchments()
            .query_by_bbox(&query_bbox)
            .unwrap_or_else(|error| panic!("seam-band catchment query must succeed: {error}"))
        {
            units.entry(unit.id()).or_insert(unit);
        }
    }
    let mut candidates = units
        .into_values()
        .filter(|unit| unit.level() == selected_level)
        .filter_map(|unit| {
            let outlet = unit.outlet();
            let native = forward(crs, GeoCoord::new(outlet.lon(), outlet.lat()));
            let containing = bands
                .iter()
                .copied()
                .filter(|band| band.contains(native))
                .collect::<Vec<_>>();
            if containing.is_empty() {
                return None;
            }
            let lies_in_x_band = containing.iter().any(|band| band.axis == SeamAxis::X);
            let lies_in_y_band = containing.iter().any(|band| band.axis == SeamAxis::Y);
            let minimum_perpendicular_distance_pixels = containing
                .iter()
                .map(|band| band.perpendicular_distance_pixels(native))
                .reduce(f64::min)
                .expect("containing band set must be nonempty");
            Some(OrderedCandidate {
                unit,
                lies_in_x_band,
                lies_in_y_band,
                minimum_perpendicular_distance_pixels,
            })
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        let left_both = left.lies_in_x_band && left.lies_in_y_band;
        let right_both = right.lies_in_x_band && right.lies_in_y_band;
        right_both
            .cmp(&left_both)
            .then_with(|| {
                left.minimum_perpendicular_distance_pixels
                    .total_cmp(&right.minimum_perpendicular_distance_pixels)
            })
            .then_with(|| left.unit.id().cmp(&right.unit.id()))
    });
    candidates
}

fn predict_pixel_window(grid: GridObservation, bbox: &Rect<f64>) -> Option<PredictedPixelWindow> {
    let min_col = ((bbox.min().x - grid.origin_x) / grid.pixel_width).floor() as i64 - 1;
    let max_col = ((bbox.max().x - grid.origin_x) / grid.pixel_width).ceil() as i64 + 1;
    let min_row = ((bbox.max().y - grid.origin_y) / grid.pixel_height).floor() as i64 - 1;
    let max_row = ((bbox.min().y - grid.origin_y) / grid.pixel_height).ceil() as i64 + 1;
    let col_off = min_col.clamp(0, i64::from(grid.raster_width)) as u32;
    let row_off = min_row.clamp(0, i64::from(grid.raster_height)) as u32;
    let col_end = max_col.clamp(0, i64::from(grid.raster_width)) as u32;
    let row_end = max_row.clamp(0, i64::from(grid.raster_height)) as u32;
    let width = col_end.saturating_sub(col_off);
    let height = row_end.saturating_sub(row_off);
    (width > 0 && height > 0).then_some(PredictedPixelWindow {
        col_off,
        row_off,
        width,
        height,
    })
}

fn pixel_bbox(
    grid: GridObservation,
    col_start: u32,
    col_end: u32,
    row_start: u32,
    row_end: u32,
) -> Rect<f64> {
    let x = sorted_pair(
        grid.origin_x + f64::from(col_start) * grid.pixel_width,
        grid.origin_x + f64::from(col_end) * grid.pixel_width,
    );
    let y = sorted_pair(
        grid.origin_y + f64::from(row_start) * grid.pixel_height,
        grid.origin_y + f64::from(row_end) * grid.pixel_height,
    );
    Rect::new(Coord { x: x.0, y: y.0 }, Coord { x: x.1, y: y.1 })
}

fn shared_half_pixel_bbox(grid: GridObservation, col_start: u32, row_start: u32) -> Rect<f64> {
    let x = sorted_pair(
        grid.origin_x + (f64::from(col_start) + 0.5) * grid.pixel_width,
        grid.origin_x + (f64::from(col_start) + 479.5) * grid.pixel_width,
    );
    let y = sorted_pair(
        grid.origin_y + (f64::from(row_start) + 0.5) * grid.pixel_height,
        grid.origin_y + (f64::from(row_start) + 479.5) * grid.pixel_height,
    );
    Rect::new(Coord { x: x.0, y: y.0 }, Coord { x: x.1, y: y.1 })
}

fn axis_label(x: bool, y: bool) -> &'static str {
    match (x, y) {
        (false, false) => "neither",
        (true, false) => "x",
        (false, true) => "y",
        (true, true) => "x+y",
    }
}

fn crossed_axes_value(x: bool, y: bool) -> CrossedTileAxes {
    match (x, y) {
        (false, false) => CrossedTileAxes::Neither,
        (true, false) => CrossedTileAxes::X,
        (false, true) => CrossedTileAxes::Y,
        (true, true) => CrossedTileAxes::XAndY,
    }
}

fn coverage_col_rows(
    coverage: &RasterWindowCoverage,
    candidate_id: i64,
    kind: RasterKind,
) -> Vec<(u32, u32)> {
    coverage
        .covered_tile_indexes()
        .iter()
        .map(|index| {
            coverage.covered_tile_col_row(*index).unwrap_or_else(|| {
                panic!(
                    "candidate {candidate_id} {kind:?} coverage index {index} must map to column/row"
                )
            })
        })
        .collect()
}

fn coverage_axes(
    coverage: &RasterWindowCoverage,
    candidate_id: i64,
    kind: RasterKind,
) -> (bool, bool) {
    let col_rows = coverage_col_rows(coverage, candidate_id, kind);
    let columns = col_rows
        .iter()
        .map(|(column, _)| *column)
        .collect::<BTreeSet<_>>();
    let rows = col_rows
        .iter()
        .map(|(_, row)| *row)
        .collect::<BTreeSet<_>>();
    (columns.len() > 1, rows.len() > 1)
}

fn choose_outlet_tile(
    grid: GridObservation,
    predicted: PredictedPixelWindow,
    outlet: NativeCoord,
    candidate_id: i64,
    kind: RasterKind,
) -> (u32, u32) {
    let fractional_col = (outlet.x() - grid.origin_x) / grid.pixel_width;
    let fractional_row = (outlet.y() - grid.origin_y) / grid.pixel_height;
    assert!(
        fractional_col.is_finite() && fractional_row.is_finite(),
        "candidate {candidate_id} {kind:?} outlet pixel coordinates must be finite"
    );
    let col = (fractional_col.floor() as i64).clamp(0, i64::from(grid.raster_width) - 1) as u32
        / grid.tile_width;
    let row = (fractional_row.floor() as i64).clamp(0, i64::from(grid.raster_height) - 1) as u32
        / grid.tile_height;
    assert!(
        predicted.covered_tile_col_rows(grid).contains(&(col, row)),
        "candidate {candidate_id} {kind:?} resolved outlet tile ({col}, {row}) must be covered by the predicted production window"
    );
    (col, row)
}

fn selected_seam(
    start: u32,
    end: u32,
    outlet_tile: u32,
    tile_size: u32,
    raster_dimension: u32,
    candidate_id: i64,
    kind: RasterKind,
    axis: &'static str,
) -> u32 {
    let lower = outlet_tile
        .checked_mul(tile_size)
        .expect("lower seam arithmetic must not overflow");
    let upper = outlet_tile
        .checked_add(1)
        .and_then(|value| value.checked_mul(tile_size))
        .expect("upper seam arithmetic must not overflow");
    let crosses_lower = lower > 0 && start < lower && end > lower;
    let crosses_upper = upper < raster_dimension && start < upper && end > upper;
    match (crosses_lower, crosses_upper) {
        (false, false) => panic!(
            "candidate {candidate_id} {kind:?} predicted {axis} window [{start}, {end}) crosses an axis but neither seam bounding outlet tile {outlet_tile}"
        ),
        (true, false) => lower,
        (false, true) | (true, true) => upper,
    }
}

fn prove_overlap(
    session: &DatasetSession,
    raster_source: &GdalRasterSource,
    handle: &D8RasterHandle,
    kind: RasterKind,
    grid: GridObservation,
    predicted: PredictedPixelWindow,
    resolved_outlet_native: NativeCoord,
    candidate_id: i64,
) -> OverlapAttempt {
    let (crosses_x, crosses_y) = predicted.crossed_axes(grid);
    assert!(
        crosses_x || crosses_y,
        "candidate {candidate_id} {kind:?} overlap requires a predicted crossed axis"
    );
    let (inner_tile_col, inner_tile_row) =
        choose_outlet_tile(grid, predicted, resolved_outlet_native, candidate_id, kind);
    let inner_col_start = inner_tile_col
        .checked_mul(TILE_SIZE)
        .and_then(|value| value.checked_add(INNER_TILE_INSET_PIXELS))
        .expect("inner column start arithmetic must not overflow");
    let inner_row_start = inner_tile_row
        .checked_mul(TILE_SIZE)
        .and_then(|value| value.checked_add(INNER_TILE_INSET_PIXELS))
        .expect("inner row start arithmetic must not overflow");
    let inner_col_end = inner_col_start
        .checked_add(INNER_SUBWINDOW_PIXELS)
        .expect("inner column end arithmetic must not overflow");
    let inner_row_end = inner_row_start
        .checked_add(INNER_SUBWINDOW_PIXELS)
        .expect("inner row end arithmetic must not overflow");
    assert!(
        inner_col_end <= grid.raster_width && inner_row_end <= grid.raster_height,
        "candidate {candidate_id} {kind:?} inner tile must contain the full 16-pixel-inset 480x480 subwindow"
    );

    let (wide_col_start, wide_col_end) = if crosses_x {
        let seam = selected_seam(
            predicted.col_off,
            predicted.col_end(),
            inner_tile_col,
            grid.tile_width,
            grid.raster_width,
            candidate_id,
            kind,
            "x",
        );
        (
            seam.checked_sub(WIDE_HALF_SPAN_PIXELS)
                .expect("wide x start must remain inside the raster"),
            seam.checked_add(WIDE_HALF_SPAN_PIXELS)
                .expect("wide x end must not overflow"),
        )
    } else {
        (inner_col_start, inner_col_end)
    };
    let (wide_row_start, wide_row_end) = if crosses_y {
        let seam = selected_seam(
            predicted.row_off,
            predicted.row_end(),
            inner_tile_row,
            grid.tile_height,
            grid.raster_height,
            candidate_id,
            kind,
            "y",
        );
        (
            seam.checked_sub(WIDE_HALF_SPAN_PIXELS)
                .expect("wide y start must remain inside the raster"),
            seam.checked_add(WIDE_HALF_SPAN_PIXELS)
                .expect("wide y end must not overflow"),
        )
    } else {
        (inner_row_start, inner_row_end)
    };
    assert!(
        wide_col_end <= grid.raster_width && wide_row_end <= grid.raster_height,
        "candidate {candidate_id} {kind:?} wide overlap request must remain inside raster bounds"
    );
    assert!(
        wide_col_start <= inner_col_start
            && inner_col_end <= wide_col_end
            && wide_row_start <= inner_row_start
            && inner_row_end <= wide_row_end,
        "candidate {candidate_id} {kind:?} inner pixel ranges must be contained in the wide requested ranges"
    );
    assert_eq!(
        wide_col_end - wide_col_start,
        if crosses_x { 992 } else { 480 },
        "candidate {candidate_id} {kind:?} wide requested width must match the crossed x-axis predicate"
    );
    assert_eq!(
        wide_row_end - wide_row_start,
        if crosses_y { 992 } else { 480 },
        "candidate {candidate_id} {kind:?} wide requested height must match the crossed y-axis predicate"
    );

    let wide_bbox = pixel_bbox(
        grid,
        wide_col_start,
        wide_col_end,
        wide_row_start,
        wide_row_end,
    );
    let inner_bbox = pixel_bbox(
        grid,
        inner_col_start,
        inner_col_end,
        inner_row_start,
        inner_row_end,
    );
    let wide = match session.localize_d8_raster_window(handle, kind, wide_bbox) {
        Ok(window) => window,
        Err(_) => return OverlapAttempt::LocalizationFailure,
    };
    let inner = match session.localize_d8_raster_window(handle, kind, inner_bbox) {
        Ok(window) => window,
        Err(_) => return OverlapAttempt::LocalizationFailure,
    };
    let wide_coverage = wide.coverage().unwrap_or_else(|| {
        panic!("candidate {candidate_id} {kind:?} wide localization must report coverage")
    });
    let inner_coverage = inner.coverage().unwrap_or_else(|| {
        panic!("candidate {candidate_id} {kind:?} inner localization must report coverage")
    });
    let wide_axes = coverage_axes(wide_coverage, candidate_id, kind);
    assert_eq!(
        wide_axes,
        (crosses_x, crosses_y),
        "candidate {candidate_id} {kind:?} wide localization must cross exactly the intended axes"
    );
    assert_eq!(
        wide_coverage.crossed_axes(),
        crossed_axes_value(wide_axes.0, wide_axes.1),
        "candidate {candidate_id} {kind:?} wide coverage classification must agree with distinct columns and rows"
    );
    let wide_col_rows = coverage_col_rows(wide_coverage, candidate_id, kind);
    let wide_columns = wide_col_rows
        .iter()
        .map(|(column, _)| *column)
        .collect::<BTreeSet<_>>();
    let wide_rows = wide_col_rows
        .iter()
        .map(|(_, row)| *row)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        wide_columns.len(),
        if crosses_x { 2 } else { 1 },
        "candidate {candidate_id} {kind:?} wide localization must not add an unexpected tile column"
    );
    assert_eq!(
        wide_rows.len(),
        if crosses_y { 2 } else { 1 },
        "candidate {candidate_id} {kind:?} wide localization must not add an unexpected tile row"
    );
    assert!(
        wide_col_rows.contains(&(inner_tile_col, inner_tile_row)),
        "candidate {candidate_id} {kind:?} wide localization must include the inner tile"
    );
    assert_eq!(
        inner_coverage.covered_tile_indexes().len(),
        1,
        "candidate {candidate_id} {kind:?} inner localization must cover exactly one tile"
    );
    assert_eq!(
        inner_coverage.crossed_axes(),
        CrossedTileAxes::Neither,
        "candidate {candidate_id} {kind:?} inner localization must cross neither axis"
    );
    let inner_tile_index = inner_coverage.covered_tile_indexes()[0];
    assert_eq!(
        inner_coverage.covered_tile_col_row(inner_tile_index),
        Some((inner_tile_col, inner_tile_row)),
        "candidate {candidate_id} {kind:?} inner coverage must map to the selected outlet tile"
    );

    let shared_bbox = shared_half_pixel_bbox(grid, inner_col_start, inner_row_start);
    match kind {
        RasterKind::FlowDir => {
            let wide_tile = match raster_source.load_flow_direction(
                &wide.path().to_string_lossy(),
                &shared_bbox,
                handle.flow_dir_encoding(),
            ) {
                Ok(tile) => tile,
                Err(_) => return OverlapAttempt::LocalizationFailure,
            };
            let inner_tile = match raster_source.load_flow_direction(
                &inner.path().to_string_lossy(),
                &shared_bbox,
                handle.flow_dir_encoding(),
            ) {
                Ok(tile) => tile,
                Err(_) => return OverlapAttempt::LocalizationFailure,
            };
            let dimensions_match = wide_tile.cols() > 0
                && wide_tile.rows() > 0
                && inner_tile.cols() > 0
                && inner_tile.rows() > 0
                && wide_tile.cols() == inner_tile.cols()
                && wide_tile.rows() == inner_tile.rows()
                && wide_tile.cols() == INNER_SUBWINDOW_PIXELS as usize
                && wide_tile.rows() == INNER_SUBWINDOW_PIXELS as usize;
            if !dimensions_match {
                return OverlapAttempt::DimensionPreconditionFailure;
            }
            assert_shared_dimensions(
                wide_tile.cols(),
                wide_tile.rows(),
                inner_tile.cols(),
                inner_tile.rows(),
                candidate_id,
                kind,
            );
            assert_shared_geotransform(wide_tile.geo(), inner_tile.geo(), candidate_id, kind);
            let wide_nodata = wide_tile.inner().nodata();
            let inner_nodata = inner_tile.inner().nodata();
            assert_eq!(
                wide_nodata, inner_nodata,
                "candidate {candidate_id} flow-direction effective nodata bytes must agree"
            );
            let mut real_sample_count = 0usize;
            for (index, (wide_value, inner_value)) in wide_tile
                .inner()
                .data()
                .iter()
                .zip(inner_tile.inner().data())
                .enumerate()
            {
                let row = index / INNER_SUBWINDOW_PIXELS as usize;
                let column = index % INNER_SUBWINDOW_PIXELS as usize;
                assert_eq!(
                    wide_value, inner_value,
                    "candidate {candidate_id} flow-direction mismatch at shared row {row} column {column}"
                );
                if *wide_value != wide_nodata {
                    real_sample_count += 1;
                }
            }
            assert_eq!(
                wide_tile.inner().data().len(),
                230_400,
                "candidate {candidate_id} flow-direction shared sample arithmetic must equal 230400"
            );
            if real_sample_count == 0 {
                return OverlapAttempt::AllNodata;
            }
            OverlapAttempt::Witness(RasterOverlapEvidence {
                wide_requested_width_pixels: wide_col_end - wide_col_start,
                wide_requested_height_pixels: wide_row_end - wide_row_start,
                wide_window_covered_tile_indexes: wide_coverage.covered_tile_indexes().to_vec(),
                wide_window_crossed_axes: axis_label(wide_axes.0, wide_axes.1),
                inner_window_covered_tile_index: inner_tile_index,
                shared_width_pixels: wide_tile.cols(),
                shared_height_pixels: wide_tile.rows(),
                real_sample_count,
                agreement: "exact_u8",
            })
        }
        RasterKind::FlowAcc => {
            for (label, path) in [("wide", wide.path()), ("inner", inner.path())] {
                let dataset = match GdalDataset::open(path) {
                    Ok(dataset) => dataset,
                    Err(_) => return OverlapAttempt::LocalizationFailure,
                };
                let band = match dataset.rasterband(1) {
                    Ok(band) => band,
                    Err(_) => return OverlapAttempt::LocalizationFailure,
                };
                assert!(
                    band.no_data_value().is_some(),
                    "candidate {candidate_id} flow-accumulation {label} source band must declare nodata"
                );
            }
            let wide_tile = match raster_source
                .load_accumulation(&wide.path().to_string_lossy(), &shared_bbox)
            {
                Ok(tile) => tile,
                Err(_) => return OverlapAttempt::LocalizationFailure,
            };
            let inner_tile = match raster_source
                .load_accumulation(&inner.path().to_string_lossy(), &shared_bbox)
            {
                Ok(tile) => tile,
                Err(_) => return OverlapAttempt::LocalizationFailure,
            };
            let dimensions_match = wide_tile.cols() > 0
                && wide_tile.rows() > 0
                && inner_tile.cols() > 0
                && inner_tile.rows() > 0
                && wide_tile.cols() == inner_tile.cols()
                && wide_tile.rows() == inner_tile.rows()
                && wide_tile.cols() == INNER_SUBWINDOW_PIXELS as usize
                && wide_tile.rows() == INNER_SUBWINDOW_PIXELS as usize;
            if !dimensions_match {
                return OverlapAttempt::DimensionPreconditionFailure;
            }
            assert_shared_dimensions(
                wide_tile.cols(),
                wide_tile.rows(),
                inner_tile.cols(),
                inner_tile.rows(),
                candidate_id,
                kind,
            );
            assert_shared_geotransform(wide_tile.geo(), inner_tile.geo(), candidate_id, kind);
            let mut real_sample_count = 0usize;
            for (index, (wide_value, inner_value)) in wide_tile
                .inner()
                .data()
                .iter()
                .zip(inner_tile.inner().data())
                .enumerate()
            {
                let row = index / INNER_SUBWINDOW_PIXELS as usize;
                let column = index % INNER_SUBWINDOW_PIXELS as usize;
                if wide_value.is_nan() || inner_value.is_nan() {
                    assert!(
                        wide_value.is_nan() && inner_value.is_nan(),
                        "candidate {candidate_id} flow-accumulation NaN mismatch at shared row {row} column {column}: wide={wide_value:?} inner={inner_value:?}"
                    );
                } else {
                    assert!(
                        wide_value.is_finite() && inner_value.is_finite(),
                        "candidate {candidate_id} flow-accumulation non-finite non-NaN value at shared row {row} column {column}: wide={wide_value:?} inner={inner_value:?}"
                    );
                    assert_eq!(
                        wide_value.to_bits(),
                        inner_value.to_bits(),
                        "candidate {candidate_id} flow-accumulation bit mismatch at shared row {row} column {column}: wide={wide_value:?} inner={inner_value:?}"
                    );
                    real_sample_count += 1;
                }
            }
            assert_eq!(
                wide_tile.inner().data().len(),
                230_400,
                "candidate {candidate_id} flow-accumulation shared sample arithmetic must equal 230400"
            );
            if real_sample_count == 0 {
                return OverlapAttempt::AllNodata;
            }
            OverlapAttempt::Witness(RasterOverlapEvidence {
                wide_requested_width_pixels: wide_col_end - wide_col_start,
                wide_requested_height_pixels: wide_row_end - wide_row_start,
                wide_window_covered_tile_indexes: wide_coverage.covered_tile_indexes().to_vec(),
                wide_window_crossed_axes: axis_label(wide_axes.0, wide_axes.1),
                inner_window_covered_tile_index: inner_tile_index,
                shared_width_pixels: wide_tile.cols(),
                shared_height_pixels: wide_tile.rows(),
                real_sample_count,
                agreement: "paired_nan_else_finite_to_bits",
            })
        }
    }
}

fn assert_shared_dimensions(
    wide_width: usize,
    wide_height: usize,
    inner_width: usize,
    inner_height: usize,
    candidate_id: i64,
    kind: RasterKind,
) {
    assert!(
        wide_width > 0 && inner_width > 0,
        "candidate {candidate_id} {kind:?} shared widths must be positive"
    );
    assert!(
        wide_height > 0 && inner_height > 0,
        "candidate {candidate_id} {kind:?} shared heights must be positive"
    );
    assert_eq!(
        wide_width, inner_width,
        "candidate {candidate_id} {kind:?} shared widths must agree"
    );
    assert_eq!(
        wide_height, inner_height,
        "candidate {candidate_id} {kind:?} shared heights must agree"
    );
    assert_eq!(
        wide_width, 480,
        "candidate {candidate_id} {kind:?} shared width must equal 480"
    );
    assert_eq!(
        wide_height, 480,
        "candidate {candidate_id} {kind:?} shared height must equal 480"
    );
}

fn assert_shared_geotransform(
    wide: &pourpoint_core::algo::GeoTransform,
    inner: &pourpoint_core::algo::GeoTransform,
    candidate_id: i64,
    kind: RasterKind,
) {
    assert_eq!(
        wide.pixel_width().to_bits(),
        inner.pixel_width().to_bits(),
        "candidate {candidate_id} {kind:?} shared pixel widths must be bitwise equal"
    );
    assert_eq!(
        wide.pixel_height().to_bits(),
        inner.pixel_height().to_bits(),
        "candidate {candidate_id} {kind:?} shared pixel heights must be bitwise equal"
    );
    assert!(
        (wide.origin_x() - inner.origin_x()).abs() <= 0.000_001 * wide.pixel_width(),
        "candidate {candidate_id} {kind:?} shared x origins exceed 1e-6 pixel tolerance: wide={} inner={} pixel_width={}",
        wide.origin_x(),
        inner.origin_x(),
        wide.pixel_width()
    );
    assert!(
        (wide.origin_y() - inner.origin_y()).abs() <= 0.000_001 * wide.pixel_height().abs(),
        "candidate {candidate_id} {kind:?} shared y origins exceed 1e-6 pixel tolerance: wide={} inner={} pixel_height={}",
        wide.origin_y(),
        inner.origin_y(),
        wide.pixel_height()
    );
}

fn terminal_window_evidence(
    coverage: &RasterWindowCoverage,
    telemetry_tile_count: u64,
    kind: RasterKind,
) -> TerminalWindowEvidence {
    assert_eq!(
        coverage.covered_tile_indexes().len() as u64,
        telemetry_tile_count,
        "{kind:?} cached coverage tile indexes must equal initial refinement telemetry tile count"
    );
    let col_rows = coverage
        .covered_tile_indexes()
        .iter()
        .map(|index| {
            coverage
                .covered_tile_col_row(*index)
                .map(|(column, row)| [column, row])
                .unwrap_or_else(|| {
                    panic!("{kind:?} cached coverage index {index} must map to column/row")
                })
        })
        .collect::<Vec<_>>();
    let columns = col_rows.iter().map(|pair| pair[0]).collect::<BTreeSet<_>>();
    let rows = col_rows.iter().map(|pair| pair[1]).collect::<BTreeSet<_>>();
    let axes = (columns.len() > 1, rows.len() > 1);
    assert_eq!(
        coverage.crossed_axes(),
        crossed_axes_value(axes.0, axes.1),
        "{kind:?} cached coverage classification must agree with distinct columns and rows"
    );
    assert!(
        axes.0 || axes.1,
        "{kind:?} production terminal coverage must witness at least one crossed axis"
    );
    let mut witnessed_axes = Vec::new();
    let mut unwitnessed_axes = Vec::new();
    if axes.0 {
        witnessed_axes.push("x");
    } else {
        unwitnessed_axes.push("x");
    }
    if axes.1 {
        witnessed_axes.push("y");
    } else {
        unwitnessed_axes.push("y");
    }
    TerminalWindowEvidence {
        tile_count: telemetry_tile_count,
        covered_tile_indexes: coverage.covered_tile_indexes().to_vec(),
        covered_tile_col_rows: col_rows,
        crossed_axes: axis_label(axes.0, axes.1),
        witnessed_axes,
        unwitnessed_axes,
    }
}

fn ceiling(observed: u64, limit: u64, label: &str) -> CeilingEvidence {
    assert!(
        observed <= limit,
        "{label} observed {observed} exceeds ceiling {limit}"
    );
    CeilingEvidence {
        observed,
        ceiling: limit,
        margin: limit - observed,
    }
}

fn execute_staged_seam_carve() {
    let cache = tempfile::tempdir().expect("fresh staged carve cache should be created");
    let _environment = EnvGuards::install(cache.path());
    let capture = CaptureLayer::default();
    tracing::subscriber::set_global_default(Registry::default().with(capture.clone()))
        .expect("one-test binary should install its tracing subscriber");
    let runtime = tokio::runtime::Runtime::new().expect("Tokio runtime should start");

    let (underlying, root, url, live_bytes, concrete_decorator, decorator) =
        runtime.block_on(async {
            let (underlying, root, url) = match DatasetSource::parse(PUBLIC_ROOT)
                .expect("public staged R2 root should parse")
            {
                DatasetSource::Remote {
                    store, root, url, ..
                } => (store, root, url),
                DatasetSource::Local(_) => panic!("public staged R2 root must parse as remote"),
            };
            let manifest_path = object_path(&root, MANIFEST_KEY);
            assert_eq!(
                manifest_path.as_ref(),
                "grit/hfx-v0.3.0/manifest.json",
                "parsed manifest object path must match the staged authority"
            );
            let live_result = underlying
                .get(&manifest_path)
                .await
                .expect("live manifest fetch should succeed");
            let live_meta = live_result.meta.clone();
            let live_bytes = live_result
                .bytes()
                .await
                .expect("live manifest bytes should be readable");
            let live_json: Value =
                serde_json::from_slice(&live_bytes).expect("live manifest should be valid JSON");
            assert_eq!(
                d8_count(&live_json),
                0,
                "live manifest must not already declare a blessed D8 raster"
            );
            let mut synthetic_json = live_json.clone();
            synthetic_json
                .get_mut("auxiliary")
                .and_then(Value::as_array_mut)
                .expect("live manifest root must contain an auxiliary array")
                .push(json!({
                    "schema": "hfx.aux.d8_raster.v2",
                    "artifacts": {
                        "flow_dir": FLOW_DIR_KEY,
                        "flow_acc": FLOW_ACC_KEY
                    },
                    "metadata": {
                        "crs": "EPSG:8857",
                        "flow_dir_encoding": "grass",
                        "flow_acc_units": "km2"
                    }
                }));
            assert_eq!(
                d8_count(&synthetic_json),
                1,
                "synthetic manifest must append exactly one D8 declaration"
            );
            let synthetic_bytes = Bytes::from(
                serde_json::to_vec(&synthetic_json).expect("synthetic manifest should serialize"),
            );
            let concrete_decorator = Arc::new(SyntheticManifestStore {
                inner: Arc::clone(&underlying),
                manifest_path,
                manifest_bytes: synthetic_bytes,
                manifest_meta: live_meta,
                state: Mutex::new(StoreState::default()),
                mutation_attempts: AtomicU64::new(0),
            });
            let decorator: Arc<dyn ObjectStore> = concrete_decorator.clone();
            (
                underlying,
                root,
                url,
                live_bytes,
                concrete_decorator,
                decorator,
            )
        });

    let search_session =
        DatasetSession::open_remote_with_store(Arc::clone(&decorator), &root, &url)
            .expect("search session should open through synthetic manifest");
    let selected_level = search_session
        .max_level()
        .expect("staged dataset must contain a finest level");
    let matching_declarations = search_session
        .auxiliary_declarations()
        .snaps
        .iter()
        .filter(|declaration| {
            declaration
                .references_levels
                .contains(&selected_level.get())
        })
        .collect::<Vec<_>>();
    let selected_declaration = matching_declarations
        .iter()
        .copied()
        .min_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| left.snap.cmp(&right.snap))
        })
        .expect("finest level must select one live snap declaration");
    assert!(
        matches!(
            selected_declaration.snap.as_str(),
            "aux/snap_segments.parquet" | "aux/snap_reaches.parquet"
        ),
        "selected declaration must be one of the two measured live snap artifacts"
    );
    assert_eq!(
        matching_declarations
            .iter()
            .filter(|candidate| {
                candidate.name == selected_declaration.name
                    && candidate.snap == selected_declaration.snap
            })
            .count(),
        1,
        "deterministic finest-level recomputation must select exactly one declaration"
    );
    let declaration_name = selected_declaration.name.clone();
    let declaration_artifact = selected_declaration.snap.clone();
    let references_levels = selected_declaration.references_levels.clone();
    let weight_semantics = selected_declaration.weight_semantics.clone();

    let engine_session =
        DatasetSession::open_remote_with_store(Arc::clone(&decorator), &root, &url)
            .expect("engine session should open through synthetic manifest");
    let engine = Engine::builder(engine_session)
        .with_raster_source(GdalRasterSource::new())
        .build();
    let raster_source = GdalRasterSource::new();
    let selected = engine
        .select_level(LevelSelection::Finest)
        .expect("finest level selection should succeed");
    assert_eq!(
        selected.level(),
        selected_level,
        "search and engine sessions must select the same finest level"
    );

    let seed_input = GeoCoord::new(8.5417, 47.3769);
    let seed_resolved = engine
        .resolve_outlet_at_level(seed_input, selected, &ResolverConfig::new())
        .expect("Zurich seed should resolve at the finest level");
    let seed_unit = search_session
        .catchments()
        .query_by_ids(&[seed_resolved.resolved().unit_id()])
        .expect("seed terminal catchment should query by ID")
        .into_iter()
        .next()
        .expect("seed terminal catchment must exist");
    let seed_terminal = decode_wkb_multi_polygon(seed_unit.geometry())
        .expect("seed terminal WKB must decode as a multipolygon");
    let seed_geographic_bbox = seed_terminal
        .bounding_rect()
        .expect("seed terminal must have a geographic bounding rectangle");
    let (seed_handle, seed_native_terminal) = search_session
        .select_d8_raster_for_terminal(&seed_terminal)
        .expect("seed terminal must select a D8 raster declaration");
    let crs = crs_from_handle(&seed_handle);
    let seed_center = forward(
        crs,
        GeoCoord::new(
            (seed_geographic_bbox.min().x + seed_geographic_bbox.max().x) / 2.0,
            (seed_geographic_bbox.min().y + seed_geographic_bbox.max().y) / 2.0,
        ),
    );
    let seed_native_bbox = seed_native_terminal
        .bounding_rect()
        .expect("seed native terminal must have a bounding rectangle");

    let search_start = concrete_decorator.snapshot();
    let seed_flow_dir = search_session
        .localize_d8_raster_window(&seed_handle, RasterKind::FlowDir, seed_native_bbox)
        .expect("seed flow-direction bbox should localize");
    let seed_flow_acc = search_session
        .localize_d8_raster_window(&seed_handle, RasterKind::FlowAcc, seed_native_bbox)
        .expect("seed flow-accumulation bbox should localize");
    let flow_dir_grid = GridObservation::from_coverage(
        "seed flow-direction",
        seed_flow_dir
            .coverage()
            .expect("seed flow-direction localization must report coverage"),
    );
    let flow_acc_grid = GridObservation::from_coverage(
        "seed flow-accumulation",
        seed_flow_acc
            .coverage()
            .expect("seed flow-accumulation localization must report coverage"),
    );

    let flow_dir_x_seams = nearest_internal_seams(flow_dir_grid, SeamAxis::X, seed_center);
    let flow_dir_y_seams = nearest_internal_seams(flow_dir_grid, SeamAxis::Y, seed_center);
    let flow_acc_x_seams = nearest_internal_seams(flow_acc_grid, SeamAxis::X, seed_center);
    let flow_acc_y_seams = nearest_internal_seams(flow_acc_grid, SeamAxis::Y, seed_center);
    for (name, seams) in [
        ("flow_dir_x", &flow_dir_x_seams),
        ("flow_dir_y", &flow_dir_y_seams),
        ("flow_acc_x", &flow_acc_x_seams),
        ("flow_acc_y", &flow_acc_y_seams),
    ] {
        assert!(
            !seams.is_empty(),
            "staged raster must furnish at least one internal {name} seam"
        );
    }
    let mut bands = Vec::new();
    bands.extend(bands_for_seams(
        flow_dir_grid,
        SeamAxis::X,
        seed_center,
        &flow_dir_x_seams,
    ));
    bands.extend(bands_for_seams(
        flow_dir_grid,
        SeamAxis::Y,
        seed_center,
        &flow_dir_y_seams,
    ));
    bands.extend(bands_for_seams(
        flow_acc_grid,
        SeamAxis::X,
        seed_center,
        &flow_acc_x_seams,
    ));
    bands.extend(bands_for_seams(
        flow_acc_grid,
        SeamAxis::Y,
        seed_center,
        &flow_acc_y_seams,
    ));
    let candidates = discover_candidates(&search_session, selected_level, crs, &bands);

    let mut candidates_tried = 0usize;
    let mut resolver_rejections = 0usize;
    let mut terminal_rejections = 0usize;
    let mut terminal_no_seam_rejections = 0usize;
    let mut localization_rejections = 0usize;
    let mut non_positive_overlap_rejections = 0usize;
    let mut flow_dir_all_nodata_rejections = 0usize;
    let mut flow_acc_all_nodata_rejections = 0usize;
    let mut chosen: Option<(
        LevelResolvedOutlet,
        [f64; 2],
        RasterOverlapEvidence,
        RasterOverlapEvidence,
    )> = None;

    for candidate in candidates.iter().take(CANDIDATE_BUDGET) {
        candidates_tried += 1;
        let candidate_outlet = candidate.unit.outlet();
        let candidate_input = GeoCoord::new(candidate_outlet.lon(), candidate_outlet.lat());
        let resolved =
            match engine.resolve_outlet_at_level(candidate_input, selected, &ResolverConfig::new())
            {
                Ok(resolved) => resolved,
                Err(_) => {
                    resolver_rejections += 1;
                    continue;
                }
            };
        let terminal_rows = match search_session
            .catchments()
            .query_by_ids(&[resolved.resolved().unit_id()])
        {
            Ok(rows) if rows.len() == 1 => rows,
            Ok(_) | Err(_) => {
                terminal_rejections += 1;
                continue;
            }
        };
        let terminal = match decode_wkb_multi_polygon(terminal_rows[0].geometry()) {
            Ok(terminal) => terminal,
            Err(_) => {
                terminal_rejections += 1;
                continue;
            }
        };
        let (handle, native_terminal) =
            match search_session.select_d8_raster_for_terminal(&terminal) {
                Ok(selection) => selection,
                Err(_) => {
                    terminal_rejections += 1;
                    continue;
                }
            };
        assert_eq!(
            crs_from_handle(&handle),
            crs,
            "candidate {} D8 handle CRS must match the observed seed grid CRS",
            resolved.resolved().unit_id().get()
        );
        let native_bbox = match native_terminal.bounding_rect() {
            Some(bbox) => bbox,
            None => {
                terminal_rejections += 1;
                continue;
            }
        };
        let flow_dir_prediction = predict_pixel_window(flow_dir_grid, &native_bbox);
        let flow_acc_prediction = predict_pixel_window(flow_acc_grid, &native_bbox);
        let (flow_dir_prediction, flow_acc_prediction) =
            match (flow_dir_prediction, flow_acc_prediction) {
                (Some(flow_dir), Some(flow_acc)) => (flow_dir, flow_acc),
                _ => {
                    terminal_no_seam_rejections += 1;
                    continue;
                }
            };
        let flow_dir_axes = flow_dir_prediction.crossed_axes(flow_dir_grid);
        let flow_acc_axes = flow_acc_prediction.crossed_axes(flow_acc_grid);
        let flow_dir_tiles = flow_dir_prediction.covered_tile_col_rows(flow_dir_grid);
        let flow_acc_tiles = flow_acc_prediction.covered_tile_col_rows(flow_acc_grid);
        if flow_dir_tiles.len() < 2
            || !(flow_dir_axes.0 || flow_dir_axes.1)
            || flow_acc_tiles.len() < 2
            || !(flow_acc_axes.0 || flow_acc_axes.1)
        {
            terminal_no_seam_rejections += 1;
            continue;
        }
        let resolved_outlet_native = forward(crs, resolved.resolved().resolved_coord());
        let flow_dir_overlap = prove_overlap(
            &search_session,
            &raster_source,
            &handle,
            RasterKind::FlowDir,
            flow_dir_grid,
            flow_dir_prediction,
            resolved_outlet_native,
            resolved.resolved().unit_id().get(),
        );
        let flow_acc_overlap = prove_overlap(
            &search_session,
            &raster_source,
            &handle,
            RasterKind::FlowAcc,
            flow_acc_grid,
            flow_acc_prediction,
            resolved_outlet_native,
            resolved.resolved().unit_id().get(),
        );
        if matches!(&flow_dir_overlap, OverlapAttempt::LocalizationFailure)
            || matches!(&flow_acc_overlap, OverlapAttempt::LocalizationFailure)
        {
            localization_rejections += 1;
            continue;
        }
        if matches!(
            &flow_dir_overlap,
            OverlapAttempt::DimensionPreconditionFailure
        ) || matches!(
            &flow_acc_overlap,
            OverlapAttempt::DimensionPreconditionFailure
        ) {
            non_positive_overlap_rejections += 1;
            continue;
        }
        let flow_dir_all_nodata = matches!(&flow_dir_overlap, OverlapAttempt::AllNodata);
        let flow_acc_all_nodata = matches!(&flow_acc_overlap, OverlapAttempt::AllNodata);
        if flow_dir_all_nodata || flow_acc_all_nodata {
            if flow_dir_all_nodata {
                flow_dir_all_nodata_rejections += 1;
            }
            if flow_acc_all_nodata {
                flow_acc_all_nodata_rejections += 1;
            }
            continue;
        }
        let (OverlapAttempt::Witness(flow_dir_overlap), OverlapAttempt::Witness(flow_acc_overlap)) =
            (flow_dir_overlap, flow_acc_overlap)
        else {
            panic!("candidate overlap classification must be exhaustive");
        };
        chosen = Some((
            resolved,
            [candidate_outlet.lon(), candidate_outlet.lat()],
            flow_dir_overlap,
            flow_acc_overlap,
        ));
        break;
    }

    let Some((resolved, selected_candidate_input_coord, flow_dir_overlap, flow_acc_overlap)) =
        chosen
    else {
        panic!(
            "staged seam search failed: candidate_budget=128 candidates_tried={candidates_tried} resolver={resolver_rejections} terminal={terminal_rejections} terminal_no_seam={terminal_no_seam_rejections} localization={localization_rejections} non_positive_overlap={non_positive_overlap_rejections} flow_dir_all_nodata={flow_dir_all_nodata_rejections} flow_acc_all_nodata={flow_acc_all_nodata_rejections}"
        );
    };

    let carve_start = concrete_decorator.snapshot();
    let (snap_id, weight, mainstem_status, distance_m, candidates_considered) =
        match resolved.resolved().method() {
            ResolutionMethod::Snap {
                strategy,
                snap_id,
                distance_m,
                weight,
                mainstem_status,
                candidates_considered,
            } => {
                assert_eq!(
                    strategy,
                    SnapStrategy::WeightFirst,
                    "default resolver must use weight-first snapping"
                );
                assert!(snap_id.get() > 0, "runtime snap ID must be positive");
                assert!(
                    weight.get().is_finite() && weight.get() >= 0.0,
                    "runtime snap weight must be finite and non-negative"
                );
                assert!(
                    distance_m.is_finite() && (0.0..=1_000.0).contains(&distance_m),
                    "runtime snap distance must be finite and within 0..=1000 metres"
                );
                assert!(
                    candidates_considered > 0,
                    "runtime snap must consider at least one candidate"
                );
                (
                    snap_id.get(),
                    weight.get(),
                    mainstem_status.map(|status| status.to_string()),
                    distance_m,
                    candidates_considered,
                )
            }
            other => panic!("selected candidate must resolve through Snap, got {other:?}"),
        };
    let upstream = engine
        .traverse_upstream_at_level(&resolved)
        .expect("selected-candidate upstream traversal should succeed");
    assert_eq!(
        upstream.terminal(),
        resolved.resolved().unit_id(),
        "traversal terminal must equal the resolved unit"
    );
    let upstream_count = upstream.upstream().unit_ids().len();
    let units = engine
        .produce_pre_merge_units(&upstream)
        .expect("selected-candidate pre-merge units should materialize");
    let refinement = engine
        .refine_terminal(
            &resolved,
            &units,
            &DelineationOptions::default().with_refinement_mode(RefinementMode::RequireD8),
        )
        .expect("required D8 refinement should succeed");
    let carve_end = concrete_decorator.snapshot();
    let (refined_polygon, _refined_outlet, _provenance) = match &refinement {
        TerminalRefinement::Applied {
            geometry,
            refined_outlet,
            provenance,
        } => (geometry.polygon(), refined_outlet, provenance),
        other => panic!("required D8 refinement must be Applied, got {other:?}"),
    };
    let terminal_unit = units.terminal_unit().unwrap_or_else(|| {
        panic!(
            "pre-merge units must contain terminal unit {}",
            units.terminal().get()
        )
    });
    let resolved_terminal_hfx_local = terminal_unit.area().get();
    let unrefined_terminal_geodesic = geodesic_area_multi(terminal_unit.geometry())
        .expect("pristine terminal geodesic area should compute")
        .as_f64();
    let refined_terminal_geodesic = geodesic_area_multi(refined_polygon)
        .expect("refined terminal geodesic area should compute")
        .as_f64();
    assert!(
        refined_terminal_geodesic > 0.0,
        "refined terminal geodesic area must be positive"
    );

    let flow_dir_object_path = object_path(&root, FLOW_DIR_KEY);
    let flow_acc_object_path = object_path(&root, FLOW_ACC_KEY);
    let search_flow_dir = carve_start
        .path(flow_dir_object_path.as_ref())
        .subtract(&search_start.path(flow_dir_object_path.as_ref()));
    let search_flow_acc = carve_start
        .path(flow_acc_object_path.as_ref())
        .subtract(&search_start.path(flow_acc_object_path.as_ref()));
    let initial_flow_dir = carve_end
        .path(flow_dir_object_path.as_ref())
        .subtract(&carve_start.path(flow_dir_object_path.as_ref()));
    let initial_flow_acc = carve_end
        .path(flow_acc_object_path.as_ref())
        .subtract(&carve_start.path(flow_acc_object_path.as_ref()));
    let initial_flow_dir_evidence = initial_flow_dir.evidence();
    let initial_flow_acc_evidence = initial_flow_acc.evidence();
    assert!(
        initial_flow_dir_evidence.payload_ranges_beyond_24507158 > 0,
        "flow-direction live carve must request tile payload beyond the complete index"
    );
    assert!(
        initial_flow_acc_evidence.payload_ranges_beyond_24507158 > 0,
        "flow-accumulation live carve must request tile payload beyond the complete index"
    );

    let frozen_trace = capture
        .state
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    assert_eq!(
        frozen_trace.event_count, 1,
        "refinement localization event must occur exactly once"
    );
    let flow_dir_telemetry = frozen_trace
        .flow_dir
        .clone()
        .expect("flow-direction telemetry fields must be present");
    let flow_acc_telemetry = frozen_trace
        .flow_acc
        .clone()
        .expect("flow-accumulation telemetry fields must be present");
    assert!(
        flow_dir_telemetry.tile_count >= 2,
        "flow-direction production terminal must cover at least two COG tiles"
    );
    assert!(
        flow_acc_telemetry.tile_count >= 2,
        "flow-accumulation production terminal must cover at least two COG tiles"
    );
    for (name, telemetry) in [
        ("flow_dir", &flow_dir_telemetry),
        ("flow_acc", &flow_acc_telemetry),
    ] {
        assert!(
            telemetry.header_bytes > 0,
            "{name} header bytes must be positive"
        );
        assert!(
            telemetry.tile_bytes > 0,
            "{name} tile bytes must be positive"
        );
        assert!(
            telemetry.window_pixels > 0,
            "{name} window pixels must be positive"
        );
    }
    let flow_dir_paths = frozen_trace
        .span_paths
        .get("raster_localize_flow_dir")
        .cloned()
        .unwrap_or_default();
    let flow_acc_paths = frozen_trace
        .span_paths
        .get("raster_localize_flow_acc")
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        flow_dir_paths.len(),
        1,
        "flow-direction localization span must record exactly one path"
    );
    assert_eq!(
        flow_acc_paths.len(),
        1,
        "flow-accumulation localization span must record exactly one path"
    );
    let internal_flow_dir_path = flow_dir_paths[0].clone();
    let internal_flow_acc_path = flow_acc_paths[0].clone();

    let flow_dir_compressed = initial_flow_dir_evidence.max_get_ranges_range_bytes;
    let flow_acc_compressed = initial_flow_acc_evidence.max_get_ranges_range_bytes;
    let flow_dir_decoded: u64 = 512 * 512;
    let flow_acc_decoded: u64 = 512 * 512 * 4;
    assert!(
        flow_dir_decoded <= MAX_DECODED_CHUNK_BYTES,
        "U8 decoded chunk must remain within its ceiling"
    );
    assert!(
        flow_acc_decoded <= MAX_DECODED_CHUNK_BYTES,
        "F32 decoded chunk must remain within its ceiling with 7340032 bytes of headroom"
    );
    let flow_dir_allocation = flow_dir_telemetry
        .window_pixels
        .checked_mul(1)
        .expect("flow-direction allocation arithmetic must not overflow");
    let flow_acc_allocation = flow_acc_telemetry
        .window_pixels
        .checked_mul(4)
        .expect("flow-accumulation allocation arithmetic must not overflow");
    let ceilings = CeilingsEvidence {
        status: "RECORDED_MEASUREMENTS_NOT_INDEPENDENT_PROOFS",
        flow_dir: RasterCeilingsEvidence {
            MAX_PLANNED_TILE_COUNT: ceiling(
                flow_dir_telemetry.tile_count,
                MAX_PLANNED_TILE_COUNT,
                "flow_dir MAX_PLANNED_TILE_COUNT",
            ),
            MAX_COMPRESSED_CHUNK_BYTES: ceiling(
                flow_dir_compressed,
                MAX_COMPRESSED_CHUNK_BYTES,
                "flow_dir MAX_COMPRESSED_CHUNK_BYTES",
            ),
            MAX_COVERED_CHUNK_BYTES: ceiling(
                flow_dir_telemetry.tile_bytes,
                MAX_COVERED_CHUNK_BYTES,
                "flow_dir MAX_COVERED_CHUNK_BYTES",
            ),
            MAX_DECODED_CHUNK_BYTES: ceiling(
                flow_dir_decoded,
                MAX_DECODED_CHUNK_BYTES,
                "flow_dir MAX_DECODED_CHUNK_BYTES",
            ),
            MAX_WINDOW_ALLOCATION_BYTES: ceiling(
                flow_dir_allocation,
                MAX_WINDOW_ALLOCATION_BYTES,
                "flow_dir MAX_WINDOW_ALLOCATION_BYTES",
            ),
        },
        flow_acc: RasterCeilingsEvidence {
            MAX_PLANNED_TILE_COUNT: ceiling(
                flow_acc_telemetry.tile_count,
                MAX_PLANNED_TILE_COUNT,
                "flow_acc MAX_PLANNED_TILE_COUNT",
            ),
            MAX_COMPRESSED_CHUNK_BYTES: ceiling(
                flow_acc_compressed,
                MAX_COMPRESSED_CHUNK_BYTES,
                "flow_acc MAX_COMPRESSED_CHUNK_BYTES",
            ),
            MAX_COVERED_CHUNK_BYTES: ceiling(
                flow_acc_telemetry.tile_bytes,
                MAX_COVERED_CHUNK_BYTES,
                "flow_acc MAX_COVERED_CHUNK_BYTES",
            ),
            MAX_DECODED_CHUNK_BYTES: ceiling(
                flow_acc_decoded,
                MAX_DECODED_CHUNK_BYTES,
                "flow_acc MAX_DECODED_CHUNK_BYTES",
            ),
            MAX_WINDOW_ALLOCATION_BYTES: ceiling(
                flow_acc_allocation,
                MAX_WINDOW_ALLOCATION_BYTES,
                "flow_acc MAX_WINDOW_ALLOCATION_BYTES",
            ),
        },
        f32_decoded_chunk_statement: "512x512x4=1048576 leaves 7340032 bytes of MAX_DECODED_CHUNK_BYTES headroom",
    };

    let retained_session =
        DatasetSession::open_remote_with_store(Arc::clone(&decorator), &root, &url)
            .expect("retained session should open");
    let (handle, native_terminal) = retained_session
        .select_d8_raster_for_terminal(terminal_unit.geometry())
        .expect("retained session should select the same D8 raster declaration");
    let native_bbox = native_terminal
        .bounding_rect()
        .expect("native selected terminal must have a bounding rectangle");
    let direct_flow_dir = retained_session
        .localize_d8_raster_window(&handle, RasterKind::FlowDir, native_bbox)
        .expect("direct flow-direction localization should hit cache");
    let direct_flow_acc = retained_session
        .localize_d8_raster_window(&handle, RasterKind::FlowAcc, native_bbox)
        .expect("direct flow-accumulation localization should hit cache");
    assert_eq!(
        direct_flow_dir.path(),
        Path::new(&internal_flow_dir_path),
        "direct cached flow-direction path must equal internal span path"
    );
    assert_eq!(
        direct_flow_acc.path(),
        Path::new(&internal_flow_acc_path),
        "direct cached flow-accumulation path must equal internal span path"
    );
    for (name, window) in [
        ("flow_dir", &direct_flow_dir),
        ("flow_acc", &direct_flow_acc),
    ] {
        assert_eq!(
            window.header_bytes(),
            0,
            "{name} cached header bytes must be zero"
        );
        assert_eq!(
            window.tile_bytes(),
            0,
            "{name} cached tile bytes must be zero"
        );
        assert_eq!(
            window.tile_count(),
            0,
            "{name} cached tile count must be zero"
        );
        assert_eq!(
            window.window_pixels(),
            0,
            "{name} cached window pixels must be zero"
        );
        assert!(
            window.coverage().is_some(),
            "{name} cached window must retain coverage"
        );
    }
    let terminal_windows = TerminalWindowsEvidence {
        flow_dir: terminal_window_evidence(
            direct_flow_dir
                .coverage()
                .expect("cached flow-direction coverage must exist"),
            flow_dir_telemetry.tile_count,
            RasterKind::FlowDir,
        ),
        flow_acc: terminal_window_evidence(
            direct_flow_acc
                .coverage()
                .expect("cached flow-accumulation coverage must exist"),
            flow_acc_telemetry.tile_count,
            RasterKind::FlowAcc,
        ),
    };
    let retained_snapshot = concrete_decorator.snapshot();
    let delta_flow_dir = retained_snapshot
        .path(flow_dir_object_path.as_ref())
        .subtract(&carve_end.path(flow_dir_object_path.as_ref()));
    let delta_flow_acc = retained_snapshot
        .path(flow_acc_object_path.as_ref())
        .subtract(&carve_end.path(flow_acc_object_path.as_ref()));
    let delta_flow_dir_evidence = delta_flow_dir.evidence();
    let delta_flow_acc_evidence = delta_flow_acc.evidence();
    assert_eq!(
        delta_flow_dir_evidence.payload_ranges_beyond_24507158, 0,
        "retained flow-direction session must not refetch tile payload"
    );
    assert_eq!(
        delta_flow_acc_evidence.payload_ranges_beyond_24507158, 0,
        "retained flow-accumulation session must not refetch tile payload"
    );

    let mut flow_dir_decoder = Decoder::new(
        File::open(direct_flow_dir.path())
            .expect("cached flow-direction file should open for decoding"),
    )
    .expect("cached flow-direction TIFF decoder should initialize");
    let (flow_dir_width, flow_dir_height) = flow_dir_decoder
        .dimensions()
        .expect("flow-direction dimensions should decode");
    let flow_dir_values = match flow_dir_decoder
        .read_image()
        .expect("flow-direction samples should decode")
    {
        DecodingResult::U8(values) => values,
        other => panic!("flow-direction cached TIFF must decode as U8, got {other:?}"),
    };
    let flow_dir_pixels = u64::from(flow_dir_width)
        .checked_mul(u64::from(flow_dir_height))
        .expect("flow-direction dimension multiplication must not overflow");
    assert_eq!(
        flow_dir_pixels, flow_dir_telemetry.window_pixels,
        "flow-direction TIFF dimensions must agree with initial telemetry"
    );
    assert!(
        flow_dir_width >= 19,
        "flow-direction runtime width must satisfy the combinatorial discriminator"
    );
    let distinct_values = flow_dir_values.iter().copied().collect::<BTreeSet<_>>();
    let nodata_255_count = flow_dir_values
        .iter()
        .filter(|&&value| value == 255)
        .count() as u64;
    let legal_grass_count = flow_dir_values
        .iter()
        .filter(|&&value| (0..=8).contains(&value) || (248..=254).contains(&value))
        .count() as u64;
    assert!(
        distinct_values.len() <= 18,
        "flow-direction distinct set {:?} exceeds the tight legal-plus-one cap",
        distinct_values
    );
    assert!(
        u128::from(legal_grass_count)
            .checked_mul(100)
            .expect("flow-direction occupancy multiplication must not overflow")
            >= u128::from(flow_dir_pixels),
        "legal non-nodata GRASS samples must occupy at least 1.0% of the window"
    );

    let mut flow_acc_decoder = Decoder::new(
        File::open(direct_flow_acc.path())
            .expect("cached flow-accumulation file should open for decoding"),
    )
    .expect("cached flow-accumulation TIFF decoder should initialize");
    let (flow_acc_width, flow_acc_height) = flow_acc_decoder
        .dimensions()
        .expect("flow-accumulation dimensions should decode");
    let flow_acc_values = match flow_acc_decoder
        .read_image()
        .expect("flow-accumulation samples should decode")
    {
        DecodingResult::F32(values) => values,
        other => panic!("flow-accumulation cached TIFF must decode as F32, got {other:?}"),
    };
    let flow_acc_pixels = u64::from(flow_acc_width)
        .checked_mul(u64::from(flow_acc_height))
        .expect("flow-accumulation dimension multiplication must not overflow");
    assert_eq!(
        flow_acc_pixels, flow_acc_telemetry.window_pixels,
        "flow-accumulation TIFF dimensions must agree with initial telemetry"
    );
    assert!(
        flow_acc_values.iter().all(|value| {
            value.is_nan() || (value.is_finite() && value.abs() < 1_000_000_000.0)
        }),
        "every accumulation sample must be NaN or finite with magnitude below 1 billion km2"
    );
    let nan_count = flow_acc_values
        .iter()
        .filter(|value| value.is_nan())
        .count() as u64;
    let non_nan_values = flow_acc_values
        .iter()
        .copied()
        .filter(|value| !value.is_nan())
        .collect::<Vec<_>>();
    let non_nan_count = non_nan_values.len() as u64;
    assert!(
        u128::from(non_nan_count)
            .checked_mul(100)
            .expect("flow-accumulation occupancy multiplication must not overflow")
            >= u128::from(flow_acc_pixels),
        "non-NaN accumulation samples must occupy at least 1.0% of the window"
    );
    let non_nan_min = non_nan_values
        .iter()
        .copied()
        .reduce(f32::min)
        .expect("flow-accumulation non-NaN minimum must exist");
    let non_nan_max = non_nan_values
        .iter()
        .copied()
        .reduce(f32::max)
        .expect("flow-accumulation non-NaN maximum must exist");

    let final_live_bytes = runtime.block_on(async {
        underlying
            .get(&object_path(&root, MANIFEST_KEY))
            .await
            .expect("final direct live manifest fetch should succeed")
            .bytes()
            .await
            .expect("final live manifest bytes should be readable")
    });
    assert_eq!(
        final_live_bytes, live_bytes,
        "live manifest bytes must remain byte-for-byte unchanged"
    );
    let final_live_json: Value =
        serde_json::from_slice(&final_live_bytes).expect("final live manifest should parse");
    assert_eq!(
        d8_count(&final_live_json),
        0,
        "live manifest must continue to lack a D8 declaration"
    );
    let mutation_attempt_count = concrete_decorator.mutation_attempts.load(Ordering::SeqCst);
    assert_eq!(
        mutation_attempt_count, 0,
        "read-only decorator must observe zero mutation attempts"
    );

    let total_flow_dir = flow_dir_pixels as f64;
    let total_flow_acc = flow_acc_pixels as f64;
    let evidence = Evidence {
        input_coord: [
            resolved.resolved().input_coord().lon,
            resolved.resolved().input_coord().lat,
        ],
        resolved_coord: [
            resolved.resolved().resolved_coord().lon,
            resolved.resolved().resolved_coord().lat,
        ],
        resolved_terminal_id: resolved.resolved().unit_id().get(),
        snap: SnapEvidence {
            method: "Snap",
            strategy: "WeightFirst",
            snap_id,
            weight,
            mainstem_status,
            distance_m,
            candidates_considered,
            declaration_name,
            declaration_artifact,
            references_levels,
            weight_semantics,
            declaration_status: "RECORDED_MEASUREMENT",
            bounds_status: "RECORDED_MEASUREMENTS_NOT_INDEPENDENT_PROOFS",
        },
        upstream_count,
        refinement: "Applied",
        route: RouteEvidence {
            public_custom_domain: "basin-delineations-public.upstream.tech",
            object_store_builder: "AmazonS3Builder::new",
            skip_signature: true,
            bogus_aws_credentials_installed: true,
            ambient_aws_credentials_consulted: false,
        },
        areas_km2: AreaEvidence {
            unrefined_terminal_geodesic,
            refined_terminal_geodesic,
            resolved_terminal_hfx_local,
            status: "DESCRIPTIVE_ONLY",
        },
        seam_search: SeamSearchEvidence {
            candidate_budget: CANDIDATE_BUDGET,
            candidates_tried,
            band_half_width_pixels: BAND_HALF_WIDTH_PIXELS as u32,
            band_half_length_pixels: BAND_HALF_LENGTH_PIXELS as u32,
            flow_dir_x_seam_coordinates: flow_dir_x_seams
                .iter()
                .map(|(_, coordinate)| *coordinate)
                .collect(),
            flow_dir_y_seam_coordinates: flow_dir_y_seams
                .iter()
                .map(|(_, coordinate)| *coordinate)
                .collect(),
            flow_acc_x_seam_coordinates: flow_acc_x_seams
                .iter()
                .map(|(_, coordinate)| *coordinate)
                .collect(),
            flow_acc_y_seam_coordinates: flow_acc_y_seams
                .iter()
                .map(|(_, coordinate)| *coordinate)
                .collect(),
            selected_candidate_input_coord,
            selected_resolved_terminal_id: resolved.resolved().unit_id().get(),
        },
        terminal_windows,
        overlap: OverlapEvidence {
            inner_subwindow_width_pixels: INNER_SUBWINDOW_PIXELS,
            inner_subwindow_height_pixels: INNER_SUBWINDOW_PIXELS,
            inner_tile_inset_pixels: INNER_TILE_INSET_PIXELS,
            minimum_safe_inset_pixels: 2,
            from_bbox_padding_pixels_per_side: 1,
            geotransform_origin_tolerance_pixels: 0.000_001,
            flow_dir: flow_dir_overlap,
            flow_acc: flow_acc_overlap,
        },
        store: StoreEvidence {
            seam_search: DeltaStoreEvidence {
                flow_dir: search_flow_dir.evidence(),
                flow_acc: search_flow_acc.evidence(),
            },
            initial_carve: InitialStoreEvidence {
                flow_dir: KeyedStoreRasterEvidence {
                    key: FLOW_DIR_KEY,
                    calls: initial_flow_dir_evidence,
                },
                flow_acc: KeyedStoreRasterEvidence {
                    key: FLOW_ACC_KEY,
                    calls: initial_flow_acc_evidence,
                },
            },
            retained_session_delta: DeltaStoreEvidence {
                flow_dir: delta_flow_dir_evidence,
                flow_acc: delta_flow_acc_evidence,
            },
            observation_unit: "ObjectStore_API_calls_not_HTTP_requests",
        },
        telemetry: TelemetryEvidence {
            event_count: frozen_trace.event_count,
            flow_dir: RasterTelemetryEvidence {
                header_bytes: flow_dir_telemetry.header_bytes,
                tile_bytes: flow_dir_telemetry.tile_bytes,
                tile_count: flow_dir_telemetry.tile_count,
                window_pixels: flow_dir_telemetry.window_pixels,
                internal_path: internal_flow_dir_path,
                direct_cached_path: direct_flow_dir.path().to_string_lossy().into_owned(),
            },
            flow_acc: RasterTelemetryEvidence {
                header_bytes: flow_acc_telemetry.header_bytes,
                tile_bytes: flow_acc_telemetry.tile_bytes,
                tile_count: flow_acc_telemetry.tile_count,
                window_pixels: flow_acc_telemetry.window_pixels,
                internal_path: internal_flow_acc_path,
                direct_cached_path: direct_flow_acc.path().to_string_lossy().into_owned(),
            },
        },
        ceilings,
        decoded: DecodedEvidence {
            flow_dir: FlowDirDecodedEvidence {
                sample_type: "U8",
                width: flow_dir_width,
                height: flow_dir_height,
                distinct_values: distinct_values.into_iter().collect(),
                nodata_255_count,
                nodata_255_fraction: nodata_255_count as f64 / total_flow_dir,
                legal_grass_non_nodata_count: legal_grass_count,
                legal_grass_non_nodata_fraction: legal_grass_count as f64 / total_flow_dir,
                distinct_cap: 18,
                distinct_cap_headroom_over_legal_plus_nodata: 1,
                minimum_legal_fraction: 0.01,
            },
            flow_acc: FlowAccDecodedEvidence {
                sample_type: "F32",
                width: flow_acc_width,
                height: flow_acc_height,
                nan_count,
                nan_fraction: nan_count as f64 / total_flow_acc,
                non_nan_count,
                non_nan_fraction: non_nan_count as f64 / total_flow_acc,
                non_nan_min,
                non_nan_max,
                magnitude_ceiling_km2: 1_000_000_000.0,
                minimum_non_nan_fraction: 0.01,
            },
            claim: "value-domain bounds remain descriptive; exact overlap agreement with positive real-sample counts is the binding staged-object oracle",
        },
        live_manifest: LiveManifestEvidence {
            byte_equal: true,
            d8_declaration_present: false,
        },
        mutation_attempt_count,
    };
    println!(
        "STAGED_R2_CARVE_EVIDENCE:{}",
        serde_json::to_string(&evidence).expect("typed evidence should serialize compactly")
    );
}

#[test]
#[ignore = "network-gated staged R2 carve proof; run POURPOINT_STAGED_R2_CARVE=1 cargo test -p pourpoint-gdal --test staged_r2_carve -- --ignored --nocapture"]
fn staged_r2_public_carve_selects_seam_candidate_and_proves_overlap() {
    assert!(
        std::env::var("POURPOINT_STAGED_R2_CARVE").as_deref() == Ok("1"),
        "POURPOINT_STAGED_R2_CARVE=1 is required"
    );

    execute_staged_seam_carve();
}
