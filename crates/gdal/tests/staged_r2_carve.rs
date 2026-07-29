//! staged_r2_carve : PublicHfxRoot × ZurichOutlet → WitnessedRequiredD8Carve
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
use geo::BoundingRect;
use object_store::path::Path as ObjectPath;
use object_store::{
    Attributes, CopyOptions, GetOptions, GetResult, GetResultPayload, ListResult, MultipartUpload,
    ObjectMeta, ObjectStore, ObjectStoreExt, PutMultipartOptions, PutOptions, PutPayload,
    PutResult, RenameOptions, Result as StoreResult,
};
use pourpoint_core::algo::{GeoCoord, geodesic_area_multi};
use pourpoint_core::session::{DatasetSession, RasterKind};
use pourpoint_core::source::DatasetSource;
use pourpoint_core::{
    DelineationOptions, Engine, LevelSelection, RefinementMode, ResolutionMethod, ResolverConfig,
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
const MAX_DECODED_CHUNK_BYTES: u64 = 1_048_576;
const MAX_WINDOW_ALLOCATION_BYTES: u64 = 1_073_741_824;

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

#[test]
#[ignore = "network-gated staged R2 carve proof; run POURPOINT_STAGED_R2_CARVE=1 cargo test -p pourpoint-gdal --test staged_r2_carve -- --ignored --nocapture"]
fn staged_r2_public_carve_reads_predictor_1_rasters() {
    assert!(
        std::env::var("POURPOINT_STAGED_R2_CARVE").as_deref() == Ok("1"),
        "POURPOINT_STAGED_R2_CARVE=1 is required"
    );

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

    let session = DatasetSession::open_remote_with_store(Arc::clone(&decorator), &root, &url)
        .expect("first staged session should open through synthetic manifest");
    let selected_level = session
        .max_level()
        .expect("staged dataset must contain a finest level");
    let matching_declarations = session
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

    let engine = Engine::builder(session)
        .with_raster_source(GdalRasterSource::new())
        .build();
    let input = GeoCoord {
        lon: 8.5417,
        lat: 47.3769,
    };
    let selected = engine
        .select_level(LevelSelection::Finest)
        .expect("finest level selection should succeed");
    let resolved = engine
        .resolve_outlet_at_level(input, selected, &ResolverConfig::new())
        .expect("Zurich outlet should resolve at the finest level");
    let (snap_id, weight, mainstem_status, distance_m, candidates_considered) =
        match &resolved.resolved().method {
            ResolutionMethod::Snap {
                strategy,
                snap_id,
                distance_m,
                weight,
                mainstem_status,
                candidates_considered,
            } => {
                assert_eq!(
                    *strategy,
                    SnapStrategy::WeightFirst,
                    "default resolver must use weight-first snapping"
                );
                assert!(snap_id.get() > 0, "runtime snap ID must be positive");
                assert!(
                    weight.get().is_finite() && weight.get() >= 0.0,
                    "runtime snap weight must be finite and non-negative"
                );
                assert!(
                    distance_m.is_finite() && (0.0..=1_000.0).contains(distance_m),
                    "runtime snap distance must be finite and within 0..=1000 metres"
                );
                assert!(
                    *candidates_considered > 0,
                    "runtime snap must consider at least one candidate"
                );
                (
                    snap_id.get(),
                    weight.get(),
                    mainstem_status.map(|status| status.to_string()),
                    *distance_m,
                    *candidates_considered,
                )
            }
            other => panic!("Zurich outlet must resolve through Snap, got {other:?}"),
        };
    let upstream = engine
        .traverse_upstream_at_level(&resolved)
        .expect("upstream traversal should succeed");
    assert_eq!(
        upstream.terminal(),
        resolved.resolved().unit_id,
        "traversal terminal must equal the resolved unit"
    );
    let upstream_count = upstream.upstream().unit_ids().len();
    let units = engine
        .produce_pre_merge_units(&upstream)
        .expect("pre-merge units should materialize");
    let refinement = engine
        .refine_terminal_placeholder(
            &resolved,
            &units,
            &DelineationOptions::default().with_refinement_mode(RefinementMode::RequireD8),
        )
        .expect("required D8 refinement should succeed");
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

    let initial_snapshot = concrete_decorator.snapshot();
    let initial_flow_dir = initial_snapshot.path(object_path(&root, FLOW_DIR_KEY).as_ref());
    let initial_flow_acc = initial_snapshot.path(object_path(&root, FLOW_ACC_KEY).as_ref());
    let initial_flow_dir_evidence = initial_flow_dir.evidence();
    let initial_flow_acc_evidence = initial_flow_acc.evidence();
    assert!(
        initial_flow_dir_evidence.payload_ranges_beyond_24507158 > 0,
        "flow-direction initial carve must request tile payload beyond the complete index"
    );
    assert!(
        initial_flow_acc_evidence.payload_ranges_beyond_24507158 > 0,
        "flow-accumulation initial carve must request tile payload beyond the complete index"
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
            telemetry.tile_count > 0,
            "{name} tile count must be positive"
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
        "F32 decoded chunk equals its ceiling and has ZERO MARGIN"
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
        f32_decoded_chunk_statement: "512x512x4=1048576 equals MAX_DECODED_CHUNK_BYTES; ZERO MARGIN",
    };

    let second_session =
        DatasetSession::open_remote_with_store(Arc::clone(&decorator), &root, &url)
            .expect("second retained session should open");
    let (handle, native_terminal) = second_session
        .select_d8_raster_for_terminal(terminal_unit.geometry())
        .expect("second session should select the same D8 raster declaration");
    let native_bbox = native_terminal
        .bounding_rect()
        .expect("native terminal must have a bounding rectangle");
    let direct_flow_dir = second_session
        .localize_d8_raster_window(&handle, RasterKind::FlowDir, native_bbox)
        .expect("direct flow-direction localization should hit cache");
    let direct_flow_acc = second_session
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
    }
    let retained_snapshot = concrete_decorator.snapshot();
    let delta_flow_dir = retained_snapshot
        .path(object_path(&root, FLOW_DIR_KEY).as_ref())
        .subtract(&initial_flow_dir);
    let delta_flow_acc = retained_snapshot
        .path(object_path(&root, FLOW_ACC_KEY).as_ref())
        .subtract(&initial_flow_acc);
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
            resolved.resolved().input_coord.lon,
            resolved.resolved().input_coord.lat,
        ],
        resolved_coord: [
            resolved.resolved().resolved_coord.lon,
            resolved.resolved().resolved_coord.lat,
        ],
        resolved_terminal_id: resolved.resolved().unit_id.get(),
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
        store: StoreEvidence {
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
            claim: "value-domain bounds falsify broad differenced or grossly mis-assembled decoding but do not provide bit-exact staged-object ground truth; U8 zero-filled unwritten regions are not discriminated",
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
