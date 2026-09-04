//! Local-current-HFX MERIT evidence recapture. No network source is supported.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use pourpoint_core::algo::{GeoCoord, SnapThreshold, canonical_wkb_multi_polygon};
use pourpoint_core::session::DatasetSession;
use pourpoint_core::{
    DelineationOptions, Engine, RefinementOutcome, ResolverConfig, SearchRadiusMetres,
};
use pourpoint_gdal::GdalRasterSource;
use serde_json::{Value, json};

#[test]
fn committed_current_merit_evidence_is_local_and_versioned() {
    let evidence: Value =
        serde_json::from_slice(include_bytes!("fixtures/merit-current/rhine-basel.json"))
            .expect("committed current MERIT evidence should be JSON");
    assert_eq!(evidence["source"], "local-current-hfx");
    assert_eq!(evidence["format_version"], "0.3.0");
    assert_eq!(evidence["adapter_version"], "0.2.0");
    assert_eq!(
        evidence["hfx_commit"],
        "5603645f91f80873e3d1cb9c236feb303def949e"
    );
    assert!(evidence.get("root").is_none());
    assert!(
        evidence["refinement"]
            .as_str()
            .is_some_and(|value| value.contains("VectorOutletQuantized"))
    );
}

#[test]
#[ignore = "requires licensed local MERIT source data and explicit blessing"]
fn recapture_rhine_basel_from_local_current_hfx() {
    let root = required_path("POURPOINT_MERIT_RECAPTURE_ROOT");
    assert_eq!(
        std::env::var("POURPOINT_MERIT_RECAPTURE_BLESS").as_deref(),
        Ok("1"),
        "set POURPOINT_MERIT_RECAPTURE_BLESS=1 to authorize writing evidence"
    );
    let output = required_path("POURPOINT_MERIT_RECAPTURE_OUTPUT");
    let hfx_commit = std::env::var("POURPOINT_MERIT_RECAPTURE_HFX_COMMIT")
        .expect("POURPOINT_MERIT_RECAPTURE_HFX_COMMIT must record the actual builder checkout");
    assert!(
        hfx_commit.len() == 40 && hfx_commit.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "HFX commit must be a full 40-character hexadecimal commit"
    );

    let manifest: Value = serde_json::from_slice(
        &fs::read(root.join("manifest.json")).expect("current HFX manifest should be readable"),
    )
    .expect("current HFX manifest should be JSON");
    assert_eq!(manifest["format_version"], "0.3.0");
    let adapter_version = manifest["adapter_version"]
        .as_str()
        .filter(|version| !version.is_empty())
        .expect("current HFX manifest should record a non-empty adapter_version");
    let schemas = manifest["auxiliary"]
        .as_array()
        .expect("auxiliary should be an array")
        .iter()
        .filter_map(|entry| entry["schema"].as_str())
        .collect::<Vec<_>>();
    assert!(schemas.contains(&"hfx.aux.snap.v2"));
    assert!(schemas.contains(&"hfx.aux.d8_raster.v2"));
    assert!(
        schemas
            .iter()
            .all(|schema| *schema != "hfx.aux.d8_raster.v1"),
        "stale D8 v1 input is not recapturable"
    );

    let session = DatasetSession::open_path(&root).expect("local current HFX should open");
    let engine = Engine::builder(session)
        .with_raster_source(GdalRasterSource::new())
        .build();
    let options = DelineationOptions::default()
        .with_resolver_config(ResolverConfig::new().with_search_radius(
            SearchRadiusMetres::new(5_000.0).expect("fixed radius should be valid"),
        ))
        .with_snap_threshold(SnapThreshold::DEFAULT);
    let result = engine
        .delineate(GeoCoord::new(7.5890, 47.5596), &options)
        .expect("Rhine/Basel local-current-HFX recapture should delineate");
    let canonical = canonical_wkb_multi_polygon(result.geometry())
        .expect("recaptured geometry should canonicalize");
    let refined_outlet = match result.refinement() {
        RefinementOutcome::Applied { refined_outlet, .. } => {
            Some(json!({"lon": refined_outlet.lon, "lat": refined_outlet.lat}))
        }
        RefinementOutcome::BestEffortSkipped { .. } | RefinementOutcome::Disabled => None,
    };
    let evidence = json!({
        "source": "local-current-hfx",
        "hfx_commit": hfx_commit,
        "adapter_version": adapter_version,
        "format_version": manifest["format_version"],
        "case": "rhine_basel",
        "input_outlet": {"lon": 7.5890, "lat": 47.5596},
        "search_radius_m": 5000.0,
        "snap_threshold_cells": SnapThreshold::DEFAULT.pixels(),
        "terminal_id": result.terminal_unit_id().get(),
        "resolved_outlet": {
            "lon": result.resolved_outlet().lon,
            "lat": result.resolved_outlet().lat,
        },
        "refined_outlet": refined_outlet,
        "refinement": format!("{:?}", result.refinement()),
        "area_km2": result.area_km2().as_f64(),
        "upstream_unit_count": result.upstream_unit_ids().len(),
        "canonical_wkb_sha256": sha256(&canonical),
    });
    fs::write(
        &output,
        serde_json::to_vec_pretty(&evidence).expect("evidence should serialize"),
    )
    .expect("blessed evidence output should be writable");
    println!("wrote {}", output.display());
}

fn required_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("{name} must be set")))
}

fn sha256(bytes: &[u8]) -> String {
    let mut child = Command::new("shasum")
        .args(["-a", "256"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("shasum should launch");
    child
        .stdin
        .take()
        .expect("shasum stdin should be piped")
        .write_all(bytes)
        .expect("canonical WKB should write to shasum");
    let output = child.wait_with_output().expect("shasum should finish");
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .expect("shasum output should be UTF-8")
        .split_whitespace()
        .next()
        .expect("shasum should emit a hash")
        .to_owned()
}
