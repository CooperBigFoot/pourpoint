//! remote_snap_export : S3DatasetRoot × GeoExtent → GeoPackage × HttpRequests.
//! An explicitly HTTP-enabled test store uses the parsed S3 root and URL.
//! Production CLI transport remains HTTPS-only; this test exercises the shared export path.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use gdal::vector::LayerAccess;
use object_store::aws::AmazonS3Builder;
use pourpoint_core::snap_targets::{SnapExtent, SnapScope, SnapTargets};
use pourpoint_core::source::DatasetSource;
use pourpoint_core::testutil::{
    DatasetBuilder, TestSnapDeclaration, TestSnapGeometry, TestSnapTarget,
};
use pourpoint_gdal::snap_export::SnapGeoPackage;

#[derive(Debug)]
struct Request {
    method: String,
    path: String,
    range: Option<String>,
}

struct SnapEndpoint {
    address: SocketAddr,
    requests: Arc<Mutex<Vec<Request>>>,
    stopped: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl SnapEndpoint {
    fn start(objects: BTreeMap<String, Vec<u8>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stopped = Arc::new(AtomicBool::new(false));
        let recorded = Arc::clone(&requests);
        let stop = Arc::clone(&stopped);
        let thread = std::thread::spawn(move || {
            for connection in listener.incoming() {
                let mut stream = connection.unwrap();
                if stop.load(Ordering::Acquire) {
                    break;
                }
                stream
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                let mut reader = BufReader::new(&stream);
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let mut parts = line.split_whitespace();
                let method = parts.next().unwrap().to_owned();
                let path = parts.next().unwrap().to_owned();
                let mut range = None;
                loop {
                    line.clear();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                    if let Some((name, value)) = line.split_once(':')
                        && name.eq_ignore_ascii_case("range")
                    {
                        range = Some(value.trim().to_owned());
                    }
                }
                drop(reader);
                recorded.lock().unwrap().push(Request {
                    method: method.clone(),
                    path: path.clone(),
                    range: range.clone(),
                });
                let Some(bytes) = objects.get(&path) else {
                    stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                    continue;
                };
                let (status, body, content_range) = if let Some(range) = range {
                    let (start, end) = range
                        .strip_prefix("bytes=")
                        .unwrap()
                        .split_once('-')
                        .unwrap();
                    let start: usize = start.parse().unwrap();
                    let end: usize = end.parse().unwrap();
                    assert!(start <= end && end < bytes.len());
                    (
                        "206 Partial Content",
                        &bytes[start..=end],
                        format!("Content-Range: bytes {start}-{end}/{}\r\n", bytes.len()),
                    )
                } else {
                    ("200 OK", bytes.as_slice(), String::new())
                };
                write!(stream, "HTTP/1.1 {status}\r\nContent-Length: {}\r\n{content_range}Accept-Ranges: bytes\r\nLast-Modified: Thu, 01 Jan 2026 00:00:00 GMT\r\nETag: \"snap-fixture\"\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                if method != "HEAD" {
                    stream.write_all(body).unwrap();
                }
            }
        });
        Self {
            address,
            requests,
            stopped,
            thread: Some(thread),
        }
    }
}

impl Drop for SnapEndpoint {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        TcpStream::connect(self.address).unwrap();
        self.thread.take().unwrap().join().unwrap();
    }
}

#[test]
fn s3_root_exports_declared_snap_path_with_http_ranges_without_unit_artifacts() {
    let (_dir, root) = DatasetBuilder::new(1)
        .with_custom_snap_declarations(vec![TestSnapDeclaration {
            name: "reach-stems".into(),
            path: "reaches.parquet".into(),
            references_levels: vec![0],
            targets: vec![
                TestSnapTarget {
                    id: 1,
                    catchment_id: 1,
                    weight: 2.0,
                    is_mainstem: true,
                    geometry: TestSnapGeometry::LineString(-1.0, 0.5, 2.0, 0.5),
                },
                TestSnapTarget {
                    id: 2,
                    catchment_id: 1,
                    weight: 1.0,
                    is_mainstem: false,
                    geometry: TestSnapGeometry::Point(20.0, 20.0),
                },
            ],
        }])
        .build();
    std::fs::create_dir_all(root.join("aux/snap")).unwrap();
    std::fs::rename(
        root.join("reaches.parquet"),
        root.join("aux/snap/reach-stems.parquet"),
    )
    .unwrap();
    let manifest_path = root.join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    manifest["auxiliary"][0]["artifacts"]["snap"] = "aux/snap/reach-stems.parquet".into();
    std::fs::write(manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    // Do not stage graph/catchments or a legacy root-level snap.parquet.
    let objects = ["manifest.json", "aux/snap/reach-stems.parquet"]
        .into_iter()
        .map(|name| {
            (
                format!("/snap-test/nested/hfx/{name}"),
                std::fs::read(root.join(name)).unwrap(),
            )
        })
        .collect();
    let endpoint = SnapEndpoint::start(objects);
    let output = root.join("remote.gpkg");
    let parsed = DatasetSource::parse("s3://snap-test/nested/hfx").unwrap();
    let DatasetSource::Remote {
        root: object_root,
        url,
        ..
    } = parsed
    else {
        panic!("supported S3 root must parse as remote");
    };
    assert_eq!(object_root.as_ref(), "nested/hfx");
    let store = AmazonS3Builder::new()
        .with_bucket_name("snap-test")
        .with_region("us-east-1")
        .with_endpoint(format!("http://{}", endpoint.address))
        .with_allow_http(true)
        .with_skip_signature(true)
        .build()
        .unwrap();
    let source = DatasetSource::Remote {
        root: object_root,
        url,
        store: Arc::new(store),
        http_stats: None,
    };
    let targets = SnapTargets::open(
        source,
        SnapScope::Bbox(SnapExtent::new(0.0, 0.0, 1.0, 1.0).unwrap()),
        None,
    )
    .unwrap();
    let destination = gdal::DriverManager::get_driver_by_name("GPKG")
        .unwrap()
        .create_vector_only(&output)
        .unwrap();
    let mut writer = SnapGeoPackage::new(destination, targets.metadata()).unwrap();
    targets.visit(|set, row| writer.write(set, row)).unwrap();
    writer.finish().unwrap();
    let dataset = gdal::Dataset::open(&output).unwrap();
    let mut lines = dataset.layer_by_name("reach-stems_lines").unwrap();
    assert_eq!(lines.spatial_ref().unwrap().auth_code().unwrap(), 4326);
    assert_eq!(lines.feature_count(), 1);
    let feature = lines.features().next().unwrap();
    let geometry = feature.geometry().unwrap();
    assert_eq!(geometry.get_point(0), (-1.0, 0.5, 0.0));
    assert_eq!(geometry.get_point(1), (2.0, 0.5, 0.0));
    assert_eq!(
        dataset
            .layer_by_name("reach-stems_points")
            .unwrap()
            .feature_count(),
        0
    );
    let requests = endpoint.requests.lock().unwrap();
    assert!(
        requests
            .iter()
            .any(|r| r.method == "GET" && r.path.ends_with("/manifest.json"))
    );
    assert!(
        requests.iter().any(|r| r.method == "GET"
            && r.path.ends_with("/aux/snap/reach-stems.parquet")
            && r.range.is_some()),
        "no real range GET recorded: {requests:?}"
    );
    assert!(
        requests.iter().all(|r| matches!(
            r.path.as_str(),
            "/snap-test/nested/hfx/manifest.json"
                | "/snap-test/nested/hfx/aux/snap/reach-stems.parquet"
        )),
        "unexpected artifact access: {requests:?}"
    );
}
