use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use pourpoint_core::algo::projection::Crs;
use pourpoint_core::algo::refine::RefinementError;
use pourpoint_core::support_claims::{
    AUXILIARY_SCHEMA_SUPPORT_CLAIMS, CORE_MANIFEST_SUPPORT_CLAIMS, D8_METADATA_SUPPORT_CLAIMS,
    DATASET_CRS_EPSG_4326, FLOW_DIRECTION_ENCODING_SUPPORT_CLAIMS, FORMAT_VERSION_V0_3_0,
    READER_SUPPORT_CLAIM_INVENTORIES, ReaderSupportClaim, ReaderSupportClaimId, ReaderSupportValue,
    d8_pair_is_compatible,
};
use pourpoint_core::testutil::DatasetBuilder;
use serde_json::{Value, json};
use tempfile::TempDir;

fn pourpoint() -> Command {
    Command::cargo_bin("pourpoint").unwrap()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EvidenceStrength {
    DeclarationDiscriminatingCompiledCli,
    ShippedCliBehaviorWithCatalogLiteralCorrespondence,
}

#[derive(Clone, Copy)]
struct ReaderSupportWitness {
    claim_id: ReaderSupportClaimId,
    evidence_test: &'static str,
    strength: EvidenceStrength,
}

const WITNESS_FORMAT_VERSION: ReaderSupportWitness = ReaderSupportWitness {
    claim_id: ReaderSupportClaimId::new("core-format-version-0.3.0"),
    evidence_test: "format_version_claim_has_shipped_cli_evidence",
    strength: EvidenceStrength::DeclarationDiscriminatingCompiledCli,
};
const WITNESS_DATASET_CRS: ReaderSupportWitness = ReaderSupportWitness {
    claim_id: ReaderSupportClaimId::new("core-dataset-crs-epsg-4326"),
    evidence_test: "dataset_crs_claim_has_shipped_cli_evidence",
    strength: EvidenceStrength::DeclarationDiscriminatingCompiledCli,
};
const WITNESS_FLOW_ESRI: ReaderSupportWitness = ReaderSupportWitness {
    claim_id: ReaderSupportClaimId::new("core-flow-dir-encoding-esri"),
    evidence_test: "esri_flow_direction_encoding_claim_has_discriminating_shipped_cli_evidence",
    strength: EvidenceStrength::DeclarationDiscriminatingCompiledCli,
};
const WITNESS_FLOW_TAUDEM: ReaderSupportWitness = ReaderSupportWitness {
    claim_id: ReaderSupportClaimId::new("core-flow-dir-encoding-taudem"),
    evidence_test: "taudem_flow_direction_encoding_claim_has_discriminating_shipped_cli_evidence",
    strength: EvidenceStrength::DeclarationDiscriminatingCompiledCli,
};
const WITNESS_FLOW_GRASS: ReaderSupportWitness = ReaderSupportWitness {
    claim_id: ReaderSupportClaimId::new("core-flow-dir-encoding-grass"),
    evidence_test: "grass_flow_direction_encoding_claim_has_discriminating_shipped_cli_evidence",
    strength: EvidenceStrength::DeclarationDiscriminatingCompiledCli,
};
const WITNESS_AUX_D8_V2: ReaderSupportWitness = ReaderSupportWitness {
    claim_id: ReaderSupportClaimId::new("aux-schema-d8-raster-v2"),
    evidence_test: "auxiliary_schema_claims_have_shipped_cli_evidence",
    strength: EvidenceStrength::DeclarationDiscriminatingCompiledCli,
};
const WITNESS_AUX_SNAP_V2: ReaderSupportWitness = ReaderSupportWitness {
    claim_id: ReaderSupportClaimId::new("aux-schema-snap-v2"),
    evidence_test: "auxiliary_schema_claims_have_shipped_cli_evidence",
    strength: EvidenceStrength::DeclarationDiscriminatingCompiledCli,
};
const WITNESS_AUX_GENERIC: ReaderSupportWitness = ReaderSupportWitness {
    claim_id: ReaderSupportClaimId::new("aux-schema-generic"),
    evidence_test: "auxiliary_schema_claims_have_shipped_cli_evidence",
    strength: EvidenceStrength::ShippedCliBehaviorWithCatalogLiteralCorrespondence,
};
const WITNESS_AUX_D8_V1: ReaderSupportWitness = ReaderSupportWitness {
    claim_id: ReaderSupportClaimId::new("aux-schema-d8-raster-v1-unsupported"),
    evidence_test: "auxiliary_schema_claims_have_shipped_cli_evidence",
    strength: EvidenceStrength::DeclarationDiscriminatingCompiledCli,
};
const WITNESS_D8_CRS_4326: ReaderSupportWitness = ReaderSupportWitness {
    claim_id: ReaderSupportClaimId::new("core-d8-crs-epsg-4326"),
    evidence_test: "geographic_d8_claims_have_shipped_cli_evidence",
    strength: EvidenceStrength::DeclarationDiscriminatingCompiledCli,
};
const WITNESS_D8_CRS_8857: ReaderSupportWitness = ReaderSupportWitness {
    claim_id: ReaderSupportClaimId::new("core-d8-crs-epsg-8857"),
    evidence_test: "projected_d8_claims_have_shipped_cli_evidence",
    strength: EvidenceStrength::DeclarationDiscriminatingCompiledCli,
};
const WITNESS_D8_CELLS: ReaderSupportWitness = ReaderSupportWitness {
    claim_id: ReaderSupportClaimId::new("core-d8-flow-acc-units-cells"),
    evidence_test: "geographic_d8_claims_have_shipped_cli_evidence",
    strength: EvidenceStrength::DeclarationDiscriminatingCompiledCli,
};
const WITNESS_D8_KM2: ReaderSupportWitness = ReaderSupportWitness {
    claim_id: ReaderSupportClaimId::new("core-d8-flow-acc-units-km2"),
    evidence_test: "projected_d8_claims_have_shipped_cli_evidence",
    strength: EvidenceStrength::DeclarationDiscriminatingCompiledCli,
};

const READER_SUPPORT_WITNESSES: &[ReaderSupportWitness] = &[
    WITNESS_FORMAT_VERSION,
    WITNESS_DATASET_CRS,
    WITNESS_FLOW_ESRI,
    WITNESS_FLOW_TAUDEM,
    WITNESS_FLOW_GRASS,
    WITNESS_AUX_D8_V2,
    WITNESS_AUX_SNAP_V2,
    WITNESS_AUX_GENERIC,
    WITNESS_AUX_D8_V1,
    WITNESS_D8_CRS_4326,
    WITNESS_D8_CRS_8857,
    WITNESS_D8_CELLS,
    WITNESS_D8_KM2,
];

fn duplicate_values<'a>(values: impl IntoIterator<Item = &'a str>) -> BTreeSet<&'a str> {
    let mut counts = BTreeMap::new();
    for value in values {
        *counts.entry(value).or_insert(0_usize) += 1;
    }
    counts
        .into_iter()
        .filter_map(|(value, count)| (count > 1).then_some(value))
        .collect()
}

fn lex_rust_source(source: &str) -> Result<Vec<String>, String> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte.is_ascii_whitespace() {
            index += 1;
        } else if bytes.get(index..index + 2) == Some(b"//") {
            index += 2;
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
        } else if bytes.get(index..index + 2) == Some(b"/*") {
            index += 2;
            let mut depth = 1_usize;
            while index < bytes.len() && depth > 0 {
                if bytes.get(index..index + 2) == Some(b"/*") {
                    depth += 1;
                    index += 2;
                } else if bytes.get(index..index + 2) == Some(b"*/") {
                    depth -= 1;
                    index += 2;
                } else {
                    index += 1;
                }
            }
            if depth != 0 {
                return Err("unterminated block comment".to_owned());
            }
        } else if byte == b'\'' || (byte == b'b' && bytes.get(index + 1) == Some(&b'\'')) {
            let literal_quote = index + usize::from(byte == b'b');
            let next = bytes.get(literal_quote + 1).copied();
            let after_next = bytes.get(literal_quote + 2).copied();
            let is_lifetime = byte == b'\''
                && next.is_some_and(|value| value == b'_' || value.is_ascii_alphabetic())
                && after_next != Some(b'\'');
            if is_lifetime {
                index += 2;
                while index < bytes.len()
                    && (bytes[index] == b'_' || bytes[index].is_ascii_alphanumeric())
                {
                    index += 1;
                }
            } else {
                index = literal_quote + 1;
                let mut escaped = false;
                let mut closed = false;
                while index < bytes.len() {
                    let current = bytes[index];
                    index += 1;
                    if escaped {
                        escaped = false;
                    } else if current == b'\\' {
                        escaped = true;
                    } else if current == b'\'' {
                        closed = true;
                        break;
                    }
                }
                if !closed {
                    return Err("unterminated character literal".to_owned());
                }
            }
        } else if byte == b'"' || (byte == b'b' && bytes.get(index + 1) == Some(&b'"')) {
            if byte == b'b' {
                index += 1;
            }
            index += 1;
            let mut escaped = false;
            let mut closed = false;
            while index < bytes.len() {
                let current = bytes[index];
                index += 1;
                if escaped {
                    escaped = false;
                } else if current == b'\\' {
                    escaped = true;
                } else if current == b'"' {
                    closed = true;
                    break;
                }
            }
            if !closed {
                return Err("unterminated string literal".to_owned());
            }
        } else if byte == b'r' || (byte == b'b' && bytes.get(index + 1) == Some(&b'r')) {
            let start = index;
            let mut cursor = index + usize::from(byte == b'b') + 1;
            let mut hashes = 0_usize;
            while bytes.get(cursor) == Some(&b'#') {
                hashes += 1;
                cursor += 1;
            }
            if bytes.get(cursor) == Some(&b'"') {
                cursor += 1;
                let terminator = format!("\"{}", "#".repeat(hashes));
                let Some(offset) = source[cursor..].find(&terminator) else {
                    return Err("unterminated raw string literal".to_owned());
                };
                index = cursor + offset + terminator.len();
            } else {
                index = start;
                let end = identifier_end(bytes, index);
                tokens.push(source[index..end].to_owned());
                index = end;
            }
        } else if byte == b'_' || byte.is_ascii_alphabetic() {
            let end = identifier_end(bytes, index);
            tokens.push(source[index..end].to_owned());
            index = end;
        } else {
            tokens.push(char::from(byte).to_string());
            index += 1;
        }
    }
    Ok(tokens)
}

fn identifier_end(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() && (bytes[index] == b'_' || bytes[index].is_ascii_alphanumeric()) {
        index += 1;
    }
    index
}

fn is_identifier_token(token: &str) -> bool {
    let mut bytes = token.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte == b'_' || byte.is_ascii_alphabetic())
        && bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
}

fn declared_test_names(tokens: &[String]) -> BTreeSet<&str> {
    let mut names = BTreeSet::new();
    let mut index = 0_usize;
    while index < tokens.len() {
        let run_start = index;
        let mut has_test = false;
        let mut has_disabling_attribute = false;
        while tokens.get(index).map(String::as_str) == Some("#")
            && tokens.get(index + 1).map(String::as_str) == Some("[")
        {
            let attribute_name = tokens.get(index + 2).map(String::as_str);
            has_test |= attribute_name == Some("test");
            has_disabling_attribute |= matches!(attribute_name, Some("cfg" | "ignore"));
            index += 2;
            let mut depth = 1_usize;
            while depth > 0 {
                match tokens.get(index).map(String::as_str) {
                    Some("[") => depth += 1,
                    Some("]") => depth -= 1,
                    Some(_) => {}
                    None => return names,
                }
                index += 1;
            }
        }
        if has_test
            && !has_disabling_attribute
            && tokens.get(index).map(String::as_str) == Some("fn")
            && tokens.get(index + 2).map(String::as_str) == Some("(")
            && let Some(name) = tokens.get(index + 1)
        {
            names.insert(name.as_str());
        }
        index = if index == run_start { index + 1 } else { index };
    }
    names
}

fn declared_claim_inventories(tokens: &[String]) -> BTreeSet<&str> {
    let mut names = BTreeSet::new();
    for (index, token) in tokens.iter().enumerate() {
        if !matches!(token.as_str(), "const" | "static") {
            continue;
        }
        let Some(name) = tokens.get(index + 1).map(String::as_str) else {
            continue;
        };
        if !is_identifier_token(name) || tokens.get(index + 2).map(String::as_str) != Some(":") {
            continue;
        }

        let mut type_index = index + 3;
        let is_reference = tokens.get(type_index).map(String::as_str) == Some("&");
        type_index += usize::from(is_reference);
        if tokens.get(type_index).map(String::as_str) != Some("[")
            || tokens.get(type_index + 1).map(String::as_str) != Some("ReaderSupportClaim")
        {
            continue;
        }

        type_index += 2;
        let has_array_length = tokens.get(type_index).map(String::as_str) == Some(";");
        if has_array_length {
            type_index += 1;
            let mut bracket_depth = 1_usize;
            while bracket_depth > 0 {
                match tokens.get(type_index).map(String::as_str) {
                    Some("[") => bracket_depth += 1,
                    Some("]") => bracket_depth -= 1,
                    Some(_) => {}
                    None => break,
                }
                type_index += 1;
            }
            if bracket_depth != 0 {
                continue;
            }
        } else if is_reference && tokens.get(type_index).map(String::as_str) == Some("]") {
            type_index += 1;
        } else {
            continue;
        }

        if tokens.get(type_index).map(String::as_str) == Some("=") {
            names.insert(name);
        }
    }
    names
}

fn aggregate_inventory_entries(tokens: &[String]) -> (Vec<&str>, BTreeSet<&str>) {
    let pattern = [
        "pub",
        "const",
        "READER_SUPPORT_CLAIM_INVENTORIES",
        ":",
        "&",
        "[",
        "&",
        "[",
        "ReaderSupportClaim",
        "]",
        "]",
        "=",
        "&",
        "[",
    ];
    let starts = tokens
        .windows(pattern.len())
        .enumerate()
        .filter_map(|(index, window)| {
            window
                .iter()
                .map(String::as_str)
                .eq(pattern)
                .then_some(index + pattern.len() - 1)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        starts.len(),
        1,
        "reader-support inventory aggregate declaration must exist exactly once"
    );

    let opening = starts[0];
    let mut index = opening + 1;
    let mut depth = 1_usize;
    let closing = loop {
        let token = tokens.get(index).unwrap_or_else(|| {
            panic!("reader-support inventory aggregate has no closing delimiter")
        });
        if token == "[" {
            depth += 1;
        } else if token == "]" {
            depth -= 1;
            if depth == 0 {
                break index;
            }
        }
        index += 1;
    };
    assert_eq!(
        tokens.get(closing + 1).map(String::as_str),
        Some(";"),
        "reader-support inventory aggregate closing delimiter must be followed by a semicolon"
    );

    let mut entries = Vec::new();
    let mut expect_identifier = true;
    for token in &tokens[opening + 1..closing] {
        if expect_identifier {
            let lexical = lex_rust_source(token).expect("aggregate entry should lex");
            assert!(
                lexical.as_slice() == [token.as_str()] && is_identifier_token(token),
                "reader-support inventory aggregate entries must be bare identifiers"
            );
            entries.push(token.as_str());
        } else {
            assert_eq!(
                token, ",",
                "reader-support inventory aggregate entries must be comma-separated"
            );
        }
        expect_identifier = !expect_identifier;
    }
    let duplicates = duplicate_values(entries.iter().copied());
    (entries, duplicates)
}

fn invoke_delineate(root: &Path, lat: &str, lon: &str) -> (bool, Value) {
    let output = pourpoint()
        .args([
            "delineate",
            "--dataset",
            root.to_str().unwrap(),
            &format!("--lat={lat}"),
            &format!("--lon={lon}"),
            "--no-refine",
            "--json",
        ])
        .output()
        .expect("failed to execute the shipped pourpoint binary");
    let stdout = String::from_utf8(output.stdout).expect("CLI stdout should be UTF-8");
    let json = serde_json::from_str(&stdout)
        .unwrap_or_else(|error| panic!("CLI stdout should be JSON: {error}\nstdout={stdout}"));
    (output.status.success(), json)
}

fn invoke_flow_encoding_refined_delineate(root: &Path) -> Value {
    let output = pourpoint()
        .args([
            "delineate",
            "--dataset",
            root.to_str().unwrap(),
            "--lat",
            "0.4166666666666667",
            "--lon",
            "0.9833333333333333",
            "--snap-radius",
            "1000",
            "--snap-threshold",
            "500",
        ])
        .output()
        .expect("failed to execute the shipped pourpoint binary");
    let stdout = String::from_utf8(output.stdout).expect("CLI stdout should be UTF-8");
    assert!(
        output.status.success(),
        "refined delineation should succeed: {stdout}"
    );
    serde_json::from_str(&stdout)
        .unwrap_or_else(|error| panic!("CLI stdout should be JSON: {error}\nstdout={stdout}"))
}

fn copy_directory(source: &Path, destination: &Path) {
    fs::create_dir(destination).expect("fixture copy directory should be creatable");
    for entry in fs::read_dir(source).expect("fixture directory should be readable") {
        let entry = entry.expect("fixture directory entry should be readable");
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if entry
            .file_type()
            .expect("fixture entry type should be readable")
            .is_dir()
        {
            copy_directory(&source_path, &destination_path);
        } else {
            fs::copy(source_path, destination_path).expect("fixture file should be copied");
        }
    }
}

fn projected_grass_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("crates/core/tests/fixtures/parity/tiny-with-aux-d8-projected-grass")
}

fn assert_rasters_are_byte_identical(source: &Path, copy: &Path) {
    for raster in [
        "aux/d8/projected/flow_dir.tif",
        "aux/d8/projected/flow_acc.tif",
    ] {
        assert_eq!(
            fs::read(source.join(raster)).expect("tracked raster should be readable"),
            fs::read(copy.join(raster)).expect("copied raster should be readable")
        );
    }
}

fn fixture_copy_with_flow_direction_encoding(declaration: &str) -> (tempfile::TempDir, PathBuf) {
    let source = projected_grass_fixture();
    let directory = tempfile::tempdir().expect("temporary fixture directory should be creatable");
    let root = directory.path().join("dataset");
    copy_directory(&source, &root);
    assert_rasters_are_byte_identical(&source, &root);

    let manifest_path = root.join("manifest.json");
    let mut manifest: Value = serde_json::from_slice(
        &fs::read(&manifest_path).expect("copied fixture manifest should be readable"),
    )
    .expect("copied fixture manifest should be valid JSON");
    manifest["auxiliary"][0]["metadata"]["flow_dir_encoding"] =
        Value::String(declaration.to_owned());
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).expect("edited manifest should serialize"),
    )
    .expect("edited fixture manifest should be writable");

    assert_rasters_are_byte_identical(&source, &root);
    (directory, root)
}

fn assert_encoding_outcome(
    declaration: &str,
    expected_refinement: Value,
    expected_area_km2: f64,
    expected_ring_vertex_counts: Value,
) {
    let (_directory, root) = fixture_copy_with_flow_direction_encoding(declaration);
    assert_encoding_outcome_at(
        &root,
        expected_refinement,
        expected_area_km2,
        expected_ring_vertex_counts,
    );
}

fn assert_encoding_outcome_at(
    root: &Path,
    expected_refinement: Value,
    expected_area_km2: f64,
    expected_ring_vertex_counts: Value,
) {
    let output = invoke_flow_encoding_refined_delineate(root);
    let features = output["features"]
        .as_array()
        .expect("CLI output should contain a feature array");
    assert_eq!(features.len(), 1);
    let feature = &features[0];
    assert_eq!(feature["properties"]["refinement"], expected_refinement);
    let actual_area_km2 = feature["properties"]["area_km2"]
        .as_f64()
        .expect("area_km2 should be numeric");
    assert!(
        (actual_area_km2 - expected_area_km2).abs() <= 0.025_f64,
        "flow-direction witness area differed from expected"
    );

    let ring_vertex_counts = Value::Array(
        feature["geometry"]["coordinates"]
            .as_array()
            .expect("geometry coordinates should contain polygons")
            .iter()
            .flat_map(|polygon| {
                polygon
                    .as_array()
                    .expect("polygon coordinates should contain rings")
                    .iter()
            })
            .map(|ring| {
                Value::from(
                    ring.as_array()
                        .expect("ring coordinates should contain vertices")
                        .len(),
                )
            })
            .collect(),
    );
    assert_eq!(ring_vertex_counts, expected_ring_vertex_counts);
}

fn invoke_refined_delineate(root: &Path, lon: &str, lat: &str) -> Value {
    invoke_successful_refinement(root, lon, lat, RefinementInvocation::Enabled)
}

fn invoke_unrefined_delineate(root: &Path, lon: &str, lat: &str) -> Value {
    invoke_successful_refinement(root, lon, lat, RefinementInvocation::Disabled)
}

enum RefinementInvocation {
    Enabled,
    Disabled,
}

fn invoke_successful_refinement(
    root: &Path,
    lon: &str,
    lat: &str,
    refinement: RefinementInvocation,
) -> Value {
    let mut command = pourpoint();
    command.args([
        "delineate",
        "--dataset",
        root.to_str().expect("fixture path should be UTF-8"),
        lon,
        lat,
        "--snap-threshold=500",
        "--json",
    ]);
    if matches!(refinement, RefinementInvocation::Disabled) {
        command.arg("--no-refine");
    }
    let output = command
        .output()
        .expect("failed to execute the shipped pourpoint binary");
    let stdout = String::from_utf8(output.stdout).expect("CLI stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("CLI stderr should be UTF-8");
    assert!(
        output.status.success(),
        "shipped CLI failed: status={}\nstderr={stderr}\nstdout={stdout}",
        output.status
    );
    let json: Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|error| panic!("CLI stdout should be JSON: {error}\nstdout={stdout}"));
    assert_eq!(json["successes"].as_array().map(Vec::len), Some(1));
    assert_eq!(json["failures"].as_array().map(Vec::len), Some(0));
    json
}

fn auxiliary_schema_support_claims() -> &'static [ReaderSupportClaim] {
    AUXILIARY_SCHEMA_SUPPORT_CLAIMS
}

fn replace_manifest_auxiliary(root: &Path, auxiliary: Value) {
    let path = root.join("manifest.json");
    let bytes = std::fs::read(&path).expect("fixture manifest should be readable");
    let mut manifest: Value =
        serde_json::from_slice(&bytes).expect("fixture manifest should be valid JSON");
    manifest["auxiliary"] = auxiliary;
    std::fs::write(
        path,
        serde_json::to_vec(&manifest).expect("edited manifest should serialize"),
    )
    .expect("edited fixture manifest should be writable");
}

fn copy_tracked_fixture(source: &Path) -> (TempDir, PathBuf) {
    fn copy_directory(source: &Path, destination: &Path) {
        std::fs::create_dir_all(destination).expect("fixture directory should be created");
        for entry in std::fs::read_dir(source).expect("fixture directory should be readable") {
            let entry = entry.expect("fixture entry should be readable");
            let source_path = entry.path();
            let destination_path = destination.join(entry.file_name());
            if source_path.is_dir() {
                copy_directory(&source_path, &destination_path);
            } else {
                std::fs::copy(&source_path, &destination_path)
                    .expect("fixture file should be copied byte-for-byte");
            }
        }
    }

    let directory = TempDir::new().expect("temporary fixture directory should be created");
    let root = directory.path().join("dataset");
    copy_directory(source, &root);
    (directory, root)
}

fn replace_manifest_field(root: &Path, field: &str, replacement: &str) {
    let path = root.join("manifest.json");
    let bytes = std::fs::read(&path).expect("fixture manifest should be readable");
    let mut manifest: Value =
        serde_json::from_slice(&bytes).expect("fixture manifest should be valid JSON");
    let value = manifest
        .get_mut(field)
        .unwrap_or_else(|| panic!("fixture manifest should contain {field}"));
    *value = Value::String(replacement.to_owned());
    std::fs::write(
        path,
        serde_json::to_vec(&manifest).expect("edited manifest should serialize"),
    )
    .expect("edited fixture manifest should be writable");
}

#[test]
fn mechanically_declared_reader_support_inventories_have_witness_correspondence() {
    let production_source = include_str!("../crates/core/src/support_claims.rs");
    let production_tokens =
        lex_rust_source(production_source).expect("reader-support production source should lex");
    let declared_inventories = declared_claim_inventories(&production_tokens);
    let (aggregate_entries, duplicate_aggregate_entries) =
        aggregate_inventory_entries(&production_tokens);
    let aggregate_entry_set = aggregate_entries.iter().copied().collect::<BTreeSet<_>>();
    let unaggregated_claim_inventories = declared_inventories
        .difference(&aggregate_entry_set)
        .copied()
        .collect::<BTreeSet<_>>();
    let aggregate_entries_without_declared_inventory = aggregate_entry_set
        .difference(&declared_inventories)
        .copied()
        .collect::<BTreeSet<_>>();
    assert!(
        unaggregated_claim_inventories.is_empty()
            && aggregate_entries_without_declared_inventory.is_empty()
            && duplicate_aggregate_entries.is_empty(),
        "reader-support inventory aggregation broken: unaggregated_claim_inventories={unaggregated_claim_inventories:?}; aggregate_entries_without_declared_inventory={aggregate_entries_without_declared_inventory:?}; duplicate_aggregate_entries={duplicate_aggregate_entries:?}"
    );

    let claims = READER_SUPPORT_CLAIM_INVENTORIES
        .iter()
        .flat_map(|inventory| inventory.iter())
        .collect::<Vec<_>>();
    let catalog_ids = claims
        .iter()
        .map(|claim| claim.id().as_str())
        .collect::<BTreeSet<_>>();
    let duplicate_catalog_ids = duplicate_values(claims.iter().map(|claim| claim.id().as_str()));
    assert!(
        duplicate_catalog_ids.is_empty(),
        "reader-support catalog contains duplicate claim IDs: {duplicate_catalog_ids:?}"
    );

    let witness_ids = READER_SUPPORT_WITNESSES
        .iter()
        .map(|witness| witness.claim_id.as_str())
        .collect::<BTreeSet<_>>();
    let duplicate_witness_ids = duplicate_values(
        READER_SUPPORT_WITNESSES
            .iter()
            .map(|witness| witness.claim_id.as_str()),
    );
    assert!(
        duplicate_witness_ids.is_empty(),
        "reader-support evidence table contains duplicate claim IDs: {duplicate_witness_ids:?}"
    );
    let claims_without_evidence_witnesses = catalog_ids
        .difference(&witness_ids)
        .copied()
        .collect::<BTreeSet<_>>();
    let witnesses_without_catalogued_claims = witness_ids
        .difference(&catalog_ids)
        .copied()
        .collect::<BTreeSet<_>>();
    assert!(
        claims_without_evidence_witnesses.is_empty()
            && witnesses_without_catalogued_claims.is_empty(),
        "reader-support claim/witness correspondence broken: claims_without_evidence_witnesses={claims_without_evidence_witnesses:?}; witnesses_without_catalogued_claims={witnesses_without_catalogued_claims:?}"
    );

    let test_source = include_str!("reader_support_claims.rs");
    let test_tokens = lex_rust_source(test_source).expect("integration-test source should lex");
    let declared_tests = declared_test_names(&test_tokens);
    let mut witnesses_without_declared_evidence_tests = BTreeSet::new();
    for witness in READER_SUPPORT_WITNESSES {
        let evidence_tokens = lex_rust_source(witness.evidence_test)
            .expect("evidence-test identifier should lex without error");
        assert_eq!(
            evidence_tokens.as_slice(),
            [witness.evidence_test],
            "evidence-test name must be one Rust identifier"
        );
        assert!(
            is_identifier_token(witness.evidence_test),
            "evidence-test name must be one Rust identifier"
        );
        let expected_strength = if witness.claim_id.as_str() == "aux-schema-generic" {
            EvidenceStrength::ShippedCliBehaviorWithCatalogLiteralCorrespondence
        } else {
            EvidenceStrength::DeclarationDiscriminatingCompiledCli
        };
        assert_eq!(
            witness.strength,
            expected_strength,
            "reader-support evidence strength mismatch for claim {}",
            witness.claim_id.as_str()
        );
        if !declared_tests.contains(witness.evidence_test) {
            witnesses_without_declared_evidence_tests.insert(witness.evidence_test);
        }
    }
    assert!(
        witnesses_without_declared_evidence_tests.is_empty(),
        "reader-support evidence-test correspondence broken: witnesses_without_declared_evidence_tests={witnesses_without_declared_evidence_tests:?}"
    );
}

#[test]
fn core_manifest_inventory_is_typed() {
    assert_eq!(CORE_MANIFEST_SUPPORT_CLAIMS.len(), 2);

    let format_version = &CORE_MANIFEST_SUPPORT_CLAIMS[0];
    assert_eq!(format_version.id().as_str(), "core-format-version-0.3.0");
    assert_eq!(format_version.canonical_declaration(), "0.3.0");
    assert!(matches!(
        format_version.value(),
        ReaderSupportValue::FormatVersion(hfx::FormatVersion::V0_3_0)
    ));

    let dataset_crs = &CORE_MANIFEST_SUPPORT_CLAIMS[1];
    assert_eq!(dataset_crs.id().as_str(), "core-dataset-crs-epsg-4326");
    assert_eq!(dataset_crs.canonical_declaration(), "EPSG:4326");
    assert!(matches!(
        dataset_crs.value(),
        ReaderSupportValue::DatasetCrs(hfx::Crs::Epsg4326)
    ));
}

#[test]
fn auxiliary_schema_inventory_is_typed() {
    assert_eq!(AUXILIARY_SCHEMA_SUPPORT_CLAIMS.len(), 4);

    let expected = [
        (
            "aux-schema-d8-raster-v2",
            "hfx.aux.d8_raster.v2",
            ReaderSupportValue::AuxiliarySchemaD8RasterV2,
        ),
        (
            "aux-schema-snap-v2",
            "hfx.aux.snap.v2",
            ReaderSupportValue::AuxiliarySchemaSnapV2,
        ),
        (
            "aux-schema-generic",
            "hfx.x.experimental.v1",
            ReaderSupportValue::AuxiliarySchemaGeneric,
        ),
        (
            "aux-schema-d8-raster-v1-unsupported",
            "hfx.aux.d8_raster.v1",
            ReaderSupportValue::AuxiliarySchemaD8RasterV1Unsupported,
        ),
    ];

    for (claim, (id, declaration, value)) in AUXILIARY_SCHEMA_SUPPORT_CLAIMS.iter().zip(expected) {
        assert_eq!(claim.id().as_str(), id);
        assert_eq!(claim.canonical_declaration(), declaration);
        assert_eq!(claim.value(), &value);
    }
}

#[test]
fn format_version_claim_has_shipped_cli_evidence() {
    assert_eq!(WITNESS_FORMAT_VERSION.claim_id, FORMAT_VERSION_V0_3_0.id());
    assert_eq!(
        FORMAT_VERSION_V0_3_0.id().as_str(),
        "core-format-version-0.3.0"
    );
    let (_directory, root) = DatasetBuilder::new(3).build();

    let (succeeded, json) = invoke_delineate(&root, "0.20", "1.70");
    assert!(succeeded, "claimed format version should succeed: {json}");
    assert_eq!(json["successes"].as_array().map(Vec::len), Some(1));

    replace_manifest_field(&root, "format_version", "0.2.1");
    let (succeeded, json) = invoke_delineate(&root, "0.20", "1.70");
    assert!(!succeeded, "unsupported format version should fail");
    assert_eq!(
        json["error"],
        "failed to open HFX dataset session: unsupported HFX format version \"0.2.1\", expected \"0.3.0\""
    );
}

#[test]
fn dataset_crs_claim_has_shipped_cli_evidence() {
    assert_eq!(WITNESS_DATASET_CRS.claim_id, DATASET_CRS_EPSG_4326.id());
    assert_eq!(
        DATASET_CRS_EPSG_4326.id().as_str(),
        "core-dataset-crs-epsg-4326"
    );
    let (_directory, root) = DatasetBuilder::new(3).build();

    let (succeeded, json) = invoke_delineate(&root, "0.20", "1.70");
    assert!(succeeded, "claimed dataset CRS should succeed: {json}");
    assert_eq!(json["successes"].as_array().map(Vec::len), Some(1));

    replace_manifest_field(&root, "crs", "EPSG:3857");
    let (succeeded, json) = invoke_delineate(&root, "0.20", "1.70");
    assert!(!succeeded, "unsupported dataset CRS should fail");
    assert_eq!(
        json["error"],
        "failed to open HFX dataset session: unsupported CRS \"EPSG:3857\", expected \"EPSG:4326\""
    );
}

#[test]
fn grass_flow_direction_encoding_claim_has_discriminating_shipped_cli_evidence() {
    let claim = &FLOW_DIRECTION_ENCODING_SUPPORT_CLAIMS[2];
    assert_eq!(WITNESS_FLOW_GRASS.claim_id, claim.id());
    assert_eq!(claim.id().as_str(), "core-flow-dir-encoding-grass");
    assert_eq!(claim.canonical_declaration(), "grass");
    assert!(matches!(
        claim.value(),
        ReaderSupportValue::FlowDirectionEncoding(hfx::FlowDirEncoding::Grass)
    ));
    assert_encoding_outcome_at(
        &projected_grass_fixture(),
        json!("applied(lon=0.986445, lat=0.416385)"),
        24986.140564067347,
        json!([13, 1289]),
    );
}

#[test]
fn d8_metadata_inventory_is_typed() {
    assert_eq!(D8_METADATA_SUPPORT_CLAIMS.len(), 4);

    let crs_4326 = &D8_METADATA_SUPPORT_CLAIMS[0];
    assert_eq!(crs_4326.id().as_str(), "core-d8-crs-epsg-4326");
    assert_eq!(crs_4326.canonical_declaration(), "EPSG:4326");
    assert!(matches!(
        crs_4326.value(),
        ReaderSupportValue::D8Crs(Crs::Epsg4326)
    ));

    let crs_8857 = &D8_METADATA_SUPPORT_CLAIMS[1];
    assert_eq!(crs_8857.id().as_str(), "core-d8-crs-epsg-8857");
    assert_eq!(crs_8857.canonical_declaration(), "EPSG:8857");
    assert!(matches!(
        crs_8857.value(),
        ReaderSupportValue::D8Crs(Crs::Epsg8857)
    ));

    let cells = &D8_METADATA_SUPPORT_CLAIMS[2];
    assert_eq!(cells.id().as_str(), "core-d8-flow-acc-units-cells");
    assert_eq!(cells.canonical_declaration(), "cells");
    assert!(matches!(
        cells.value(),
        ReaderSupportValue::D8FlowAccumulationUnits(hfx::FlowAccumulationUnits::Cells)
    ));

    let km2 = &D8_METADATA_SUPPORT_CLAIMS[3];
    assert_eq!(km2.id().as_str(), "core-d8-flow-acc-units-km2");
    assert_eq!(km2.canonical_declaration(), "km2");
    assert!(matches!(
        km2.value(),
        ReaderSupportValue::D8FlowAccumulationUnits(hfx::FlowAccumulationUnits::Km2)
    ));

    assert!(d8_pair_is_compatible("EPSG:4326", "cells"));
    assert!(!d8_pair_is_compatible("EPSG:4326", "km2"));
    assert!(d8_pair_is_compatible("EPSG:8857", "cells"));
    assert!(d8_pair_is_compatible("EPSG:8857", "km2"));
    assert!(!d8_pair_is_compatible("epsg:4326", "cells"));
    assert!(!d8_pair_is_compatible("EPSG:8857", "KM2"));
}

#[test]
fn geographic_d8_claims_have_shipped_cli_evidence() {
    assert_eq!(
        WITNESS_D8_CRS_4326.claim_id,
        D8_METADATA_SUPPORT_CLAIMS[0].id()
    );
    assert_eq!(
        WITNESS_D8_CELLS.claim_id,
        D8_METADATA_SUPPORT_CLAIMS[2].id()
    );
    let source = Path::new("crates/core/tests/fixtures/parity/v021_synthetic_refined");
    let source_refined = invoke_refined_delineate(source, "--lon=2.5", "--lat=-2.5");
    let source_unrefined = invoke_unrefined_delineate(source, "--lon=2.5", "--lat=-2.5");
    assert_ne!(
        source_refined, source_unrefined,
        "claimed geographic/cells declaration must refine through the shipped CLI"
    );

    assert_eq!(
        D8_METADATA_SUPPORT_CLAIMS[0].id().as_str(),
        "core-d8-crs-epsg-4326"
    );
    assert_eq!(
        D8_METADATA_SUPPORT_CLAIMS[2].id().as_str(),
        "core-d8-flow-acc-units-cells"
    );
    assert_eq!(
        D8_METADATA_SUPPORT_CLAIMS[3].id().as_str(),
        "core-d8-flow-acc-units-km2"
    );

    let directory = tempfile::tempdir().expect("temporary fixture directory should be created");
    let copied = directory.path();
    for name in [
        "manifest.json",
        "catchments.parquet",
        "graph.parquet",
        "flow_dir.tif",
        "flow_acc.tif",
    ] {
        std::fs::copy(source.join(name), copied.join(name))
            .unwrap_or_else(|error| panic!("fixture file {name} should copy: {error}"));
    }

    let source_flow_dir = std::fs::read(source.join("flow_dir.tif")).expect("source flow_dir");
    let source_flow_acc = std::fs::read(source.join("flow_acc.tif")).expect("source flow_acc");
    assert_eq!(
        std::fs::read(copied.join("flow_dir.tif")).expect("copied flow_dir"),
        source_flow_dir
    );
    assert_eq!(
        std::fs::read(copied.join("flow_acc.tif")).expect("copied flow_acc"),
        source_flow_acc
    );

    let source_manifest: Value = serde_json::from_slice(
        &std::fs::read(source.join("manifest.json")).expect("source manifest should be readable"),
    )
    .expect("source manifest should be valid JSON");
    let copied_manifest_path = copied.join("manifest.json");
    let copied_manifest: Value = serde_json::from_slice(
        &std::fs::read(&copied_manifest_path).expect("copied manifest should be readable"),
    )
    .expect("copied manifest should be valid JSON");
    assert_eq!(
        copied_manifest.pointer("/auxiliary/0/metadata/flow_acc_units"),
        Some(&Value::String("cells".to_owned()))
    );
    std::fs::write(
        &copied_manifest_path,
        serde_json::to_vec(&copied_manifest).expect("copied manifest should serialize"),
    )
    .expect("copied manifest should be writable");
    let round_tripped: Value = serde_json::from_slice(
        &std::fs::read(&copied_manifest_path).expect("round-tripped manifest should be readable"),
    )
    .expect("round-tripped manifest should be valid JSON");
    assert_eq!(round_tripped, source_manifest);

    let copied_cells_refined = invoke_refined_delineate(copied, "--lon=2.5", "--lat=-2.5");
    let copied_cells_unrefined = invoke_unrefined_delineate(copied, "--lon=2.5", "--lat=-2.5");
    assert_ne!(
        copied_cells_refined, copied_cells_unrefined,
        "claimed geographic/cells declaration must refine through the shipped CLI"
    );

    let mut geographic_km2 = round_tripped;
    *geographic_km2
        .pointer_mut("/auxiliary/0/metadata/flow_acc_units")
        .expect("flow_acc_units should exist") = Value::String("km2".to_owned());
    std::fs::write(
        &copied_manifest_path,
        serde_json::to_vec(&geographic_km2).expect("edited manifest should serialize"),
    )
    .expect("edited manifest should be writable");
    let reread_geographic_km2: Value = serde_json::from_slice(
        &std::fs::read(&copied_manifest_path).expect("edited manifest should be readable"),
    )
    .expect("edited manifest should be valid JSON");
    let mut expected = source_manifest;
    *expected
        .pointer_mut("/auxiliary/0/metadata/flow_acc_units")
        .expect("flow_acc_units should exist") = Value::String("km2".to_owned());
    assert_eq!(reread_geographic_km2, expected);
    assert_eq!(
        reread_geographic_km2.pointer("/auxiliary/0/metadata/flow_acc_units"),
        Some(&Value::String("km2".to_owned()))
    );
    assert_eq!(
        std::fs::read(copied.join("flow_dir.tif")).expect("copied flow_dir after writes"),
        source_flow_dir
    );
    assert_eq!(
        std::fs::read(copied.join("flow_acc.tif")).expect("copied flow_acc after writes"),
        source_flow_acc
    );

    let rejected = invoke_refined_delineate(copied, "--lon=2.5", "--lat=-2.5");
    assert_eq!(
        rejected, copied_cells_unrefined,
        "rejected geographic/km2 pair must match the no-refine control"
    );
    let diagnostic = RefinementError::GeographicKm2Unsupported {
        epsg: 4326,
        units: hfx::FlowAccumulationUnits::Km2,
    };
    assert_eq!(
        diagnostic.to_string(),
        "flow accumulation units km2 require projected pixel area, but EPSG:4326 is geographic"
    );
}

#[test]
fn taudem_flow_direction_encoding_claim_has_discriminating_shipped_cli_evidence() {
    let claim = &FLOW_DIRECTION_ENCODING_SUPPORT_CLAIMS[1];
    assert_eq!(WITNESS_FLOW_TAUDEM.claim_id, claim.id());
    assert_eq!(claim.id().as_str(), "core-flow-dir-encoding-taudem");
    assert_eq!(claim.canonical_declaration(), "taudem");
    assert!(matches!(
        claim.value(),
        ReaderSupportValue::FlowDirectionEncoding(hfx::FlowDirEncoding::Taudem)
    ));
    assert_encoding_outcome(
        "taudem",
        json!("applied(lon=0.986445, lat=0.416385)"),
        24613.14053443639,
        json!([13, 13]),
    );
}

#[test]
fn esri_flow_direction_encoding_claim_has_discriminating_shipped_cli_evidence() {
    let claim = &FLOW_DIRECTION_ENCODING_SUPPORT_CLAIMS[0];
    assert_eq!(WITNESS_FLOW_ESRI.claim_id, claim.id());
    assert_eq!(claim.id().as_str(), "core-flow-dir-encoding-esri");
    assert_eq!(claim.canonical_declaration(), "esri");
    assert!(matches!(
        claim.value(),
        ReaderSupportValue::FlowDirectionEncoding(hfx::FlowDirEncoding::Esri)
    ));
    assert_encoding_outcome(
        "esri",
        json!(
            "best_effort_skipped(BestEffortSkipped { strategy: BestEffortD8IfPresent, why: MisDeclaration { source: RasterLoad, diagnostic: \"flow-direction nodata byte 128 decodes as a legal direction under Esri encoding\" } })"
        ),
        36922.8059387193,
        json!([25]),
    );
}

#[test]
fn projected_d8_claims_have_shipped_cli_evidence() {
    let source = Path::new("crates/core/tests/fixtures/parity/tiny-with-aux-d8-projected-grass");
    assert_eq!(
        WITNESS_D8_CRS_8857.claim_id,
        D8_METADATA_SUPPORT_CLAIMS[1].id()
    );
    assert_eq!(WITNESS_D8_KM2.claim_id, D8_METADATA_SUPPORT_CLAIMS[3].id());
    assert_eq!(
        D8_METADATA_SUPPORT_CLAIMS[1].id().as_str(),
        "core-d8-crs-epsg-8857"
    );
    assert_eq!(
        D8_METADATA_SUPPORT_CLAIMS[3].id().as_str(),
        "core-d8-flow-acc-units-km2"
    );
    let refined = invoke_refined_delineate(
        source,
        "--lon=0.9833333333333333",
        "--lat=0.4166666666666667",
    );
    let unrefined = invoke_unrefined_delineate(
        source,
        "--lon=0.9833333333333333",
        "--lat=0.4166666666666667",
    );
    assert_ne!(
        refined, unrefined,
        "claimed projected/km2 declaration must refine through the shipped CLI"
    );
    assert_eq!(refined["successes"][0]["terminal_unit_id"], 4);
}

#[test]
fn auxiliary_schema_claims_have_shipped_cli_evidence() {
    for (witness, claim) in [
        WITNESS_AUX_D8_V2,
        WITNESS_AUX_SNAP_V2,
        WITNESS_AUX_GENERIC,
        WITNESS_AUX_D8_V1,
    ]
    .iter()
    .zip(AUXILIARY_SCHEMA_SUPPORT_CLAIMS)
    {
        assert_eq!(witness.claim_id, claim.id());
    }
    let mut calls_completed = 0;

    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("crates/core/tests/fixtures/parity/v021_synthetic_refined");
    let (_directory, root) = copy_tracked_fixture(&fixture);
    let (succeeded, json) = invoke_delineate(&root, "-2.5", "2.5");
    calls_completed += 1;
    assert!(succeeded, "tracked D8-v2 fixture should succeed: {json}");
    assert_eq!(json["succeeded"], 1);
    assert_eq!(json["failed"], 0);
    assert_eq!(json["successes"].as_array().map(Vec::len), Some(1));

    let (_directory, root) = DatasetBuilder::new(3).with_snap().build();
    let (succeeded, json) = invoke_delineate(&root, "0.20", "1.70");
    calls_completed += 1;
    assert!(succeeded, "snap-v2 fixture should succeed: {json}");
    assert_eq!(json["succeeded"], 1);
    assert_eq!(json["failed"], 0);
    assert_eq!(json["successes"].as_array().map(Vec::len), Some(1));

    let (_directory, root) = DatasetBuilder::new(3).build();
    replace_manifest_auxiliary(
        &root,
        json!([{
            "schema": "hfx.x.experimental.v1",
            "artifacts": { "x": "graph.parquet" },
            "metadata": { "literal": true }
        }]),
    );
    let (succeeded, json) = invoke_delineate(&root, "0.20", "1.70");
    calls_completed += 1;
    assert!(
        succeeded,
        "present provisional artifact should succeed: {json}"
    );
    assert_eq!(json["succeeded"], 1);
    assert_eq!(json["failed"], 0);
    assert_eq!(json["successes"].as_array().map(Vec::len), Some(1));

    replace_manifest_auxiliary(
        &root,
        json!([{
            "schema": "hfx.x.experimental.v1",
            "artifacts": { "x": "missing.bin" },
            "metadata": { "literal": true }
        }]),
    );
    let (succeeded, json) = invoke_delineate(&root, "0.20", "1.70");
    calls_completed += 1;
    assert!(!succeeded, "missing provisional artifact should fail");
    assert_eq!(
        json["error"],
        format!(
            "failed to open HFX dataset session: auxiliary artifact \"x\" for schema \"hfx.x.experimental.v1\" not found at {}",
            root.join("missing.bin").display()
        )
    );

    let (_directory, root) = DatasetBuilder::new(3).build();
    replace_manifest_auxiliary(
        &root,
        json!([{
            "schema": "com.example.thing.v1",
            "artifacts": { "x": "graph.parquet" },
            "metadata": { "literal": true }
        }]),
    );
    let (succeeded, json) = invoke_delineate(&root, "0.20", "1.70");
    calls_completed += 1;
    assert!(
        succeeded,
        "present third-party artifact should succeed: {json}"
    );
    assert_eq!(json["succeeded"], 1);
    assert_eq!(json["failed"], 0);
    assert_eq!(json["successes"].as_array().map(Vec::len), Some(1));

    replace_manifest_auxiliary(
        &root,
        json!([{
            "schema": "com.example.thing.v1",
            "artifacts": { "x": "missing.bin" },
            "metadata": { "literal": true }
        }]),
    );
    let (succeeded, json) = invoke_delineate(&root, "0.20", "1.70");
    calls_completed += 1;
    assert!(!succeeded, "missing third-party artifact should fail");
    assert_eq!(
        json["error"],
        format!(
            "failed to open HFX dataset session: auxiliary artifact \"x\" for schema \"com.example.thing.v1\" not found at {}",
            root.join("missing.bin").display()
        )
    );

    let (_directory, root) = DatasetBuilder::new(3).build();
    replace_manifest_auxiliary(
        &root,
        json!([{
            "schema": "hfx.aux.d8_raster.v1",
            "artifacts": { "x": "missing.bin" },
            "metadata": { "literal": true }
        }]),
    );
    let (succeeded, json) = invoke_delineate(&root, "0.20", "1.70");
    calls_completed += 1;
    assert!(!succeeded, "D8-v1 declaration should fail");
    assert_eq!(
        json["error"],
        "failed to open HFX dataset session: auxiliary schema \"hfx.aux.d8_raster.v1\" is no longer supported; recompile the dataset with a v2-emitting adapter that declares \"hfx.aux.d8_raster.v2\""
    );

    let (_directory, root) = DatasetBuilder::new(3).build();
    replace_manifest_auxiliary(
        &root,
        json!([{
            "schema": "hfx.aux.bogus.v9",
            "artifacts": { "x": "graph.parquet" },
            "metadata": { "literal": true }
        }]),
    );
    let (succeeded, json) = invoke_delineate(&root, "0.20", "1.70");
    calls_completed += 1;
    assert!(!succeeded, "unblessed HFX declaration should fail");
    assert_eq!(
        json["error"],
        "failed to open HFX dataset session: auxiliary declaration for schema \"hfx.aux.bogus.v9\" is invalid: malformed auxiliary schema id: \"hfx.aux.bogus.v9\""
    );

    assert_eq!(calls_completed, 8, "all shipped CLI calls must complete");
    assert_eq!(
        auxiliary_schema_support_claims().len(),
        4,
        "auxiliary-schema inventory must contain exactly four rows"
    );
    assert_eq!(
        auxiliary_schema_support_claims()[2].canonical_declaration(),
        "hfx.x.experimental.v1",
        "the generic claim representative must correspond to shipped evidence"
    );
}
