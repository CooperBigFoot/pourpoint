//! released_wheel_proof : OfflineSyntheticFixtures → NamedGuardRejections

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const PASS_LINES: [&str; 37] = [
    "PASS: an accepted seed is ranked and attempted within the supplied budget",
    "PASS: authorization failures are loud and typed",
    "PASS: live released-wheel inputs use only the exact hosted dataset base",
    "PASS: hosted worker trace, not idle proxy traffic, satisfies the worker read gate",
    "PASS: preflight reads cannot satisfy the released-worker read guard",
    "PASS: hosted ranges and allocation ceilings retain positive margin",
    "PASS: candidate exhaustion retains one reason per ordered candidate",
    "PASS: preflight requires the published D8 declaration authority",
    "PASS: seed probe uses the live measurement predicate",
    "PASS: rejected seed is recorded and ordered candidate search continues",
    "PASS: rejected seed cannot re-enter the ordered candidate loop",
    "PASS: hosted transport requires an explicit non-default User-Agent",
    "PASS: an unresolvable candidate is recorded and selection continues",
    "PASS: unresolvable discovery seed is named while ranked rejection continues",
    "PASS: unresolved status requires the worker protocol marker",
    "PASS: ambient live-input probe recorded zero reads across all self-tests",
    "PASS: every self-test result is unchanged under ambient live inputs",
    "PASS: network opener is unreachable offline",
    "PASS: missing authorization rejected loudly",
    "PASS: authorized zero completed reads rejected loudly",
    "PASS: candidate byte drift rejected",
    "PASS: candidate trailing newline drift rejected",
    "PASS: negative declaration extra drift rejected",
    "PASS: wheel size digest and metadata drift rejected",
    "PASS: mutation methods rejected",
    "PASS: unbounded COG reads rejected",
    "PASS: range ceilings rejected",
    "PASS: completed range transcript corruption rejected",
    "PASS: RequireD8 degraded status rejected",
    "PASS: missing production trace rejected",
    "PASS: production window corruption rejected",
    "PASS: canonical WKB corruption rejected",
    "PASS: negative discriminator equality rejected",
    "PASS: distant-region threshold rejected",
    "PASS: synthetic evidence cannot satisfy live proof",
    "PASS: artifact index tampering rejected",
    "PASS: historical artifacts are immutable",
];

const CLEARED_ENV: [&str; 12] = [
    "POURPOINT_LIVE_READ_AUTHORIZATION",
    "POURPOINT_RELEASE_WHEEL",
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "AWS_PROFILE",
    "AWS_SHARED_CREDENTIALS_FILE",
    "AWS_CONFIG_FILE",
    "AWS_WEB_IDENTITY_TOKEN_FILE",
    "AWS_ROLE_ARN",
    "AWS_CONTAINER_CREDENTIALS_RELATIVE_URI",
    "AWS_CONTAINER_CREDENTIALS_FULL_URI",
];

fn run(script: &str, argument: &str) -> Output {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut command = Command::new("python3");
    command
        .current_dir(root)
        .arg(root.join(script))
        .arg(argument)
        .env("POURPOINT_PROOF_NETWORK_DENIED", "1");
    for name in CLEARED_ENV {
        command.env_remove(name);
    }
    command.output().unwrap_or_else(|error| {
        panic!("failed to launch {script}: {error}");
    })
}

fn assert_success(output: &Output, script: &str) -> Vec<String> {
    assert_eq!(
        output.status.code(),
        Some(0),
        "{script} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "{script} wrote stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone())
        .unwrap_or_else(|error| panic!("{script} stdout was not UTF-8: {error}"))
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn offline_self_tests_reject_every_named_corruption() {
    let historical_paths = [
        "crates/core/tests/fixtures/parity/goldens/v01_grit_nonrefined/oracle_a_grit_nonrefined.json",
        "crates/core/tests/fixtures/parity/goldens/v01_merit_refined/oracle_c_merit_refined.json",
        "crates/core/tests/fixtures/parity/goldens/v021_synthetic_nonrefined/v021_synthetic_nonrefined.json",
        "docs/evidence/2026-08-06-released-reader-mutation-control.json",
    ];
    let before = historical_paths.map(read_bytes);

    let harness = run("scripts/released_wheel_proof.py", "self-test");
    let verifier = run("scripts/verify_released_wheel_evidence.py", "--self-test");
    let mut lines = assert_success(&harness, "released_wheel_proof.py");
    lines.extend(assert_success(
        &verifier,
        "verify_released_wheel_evidence.py",
    ));

    assert_eq!(lines, PASS_LINES);
    let unique: BTreeSet<&str> = lines.iter().map(String::as_str).collect();
    assert_eq!(unique.len(), PASS_LINES.len(), "duplicate PASS lines");

    let after = historical_paths.map(read_bytes);
    assert_eq!(
        before, after,
        "historical artifacts changed during self-test"
    );
}

fn read_bytes(relative: &str) -> Vec<u8> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    std::fs::read(root.join(relative))
        .unwrap_or_else(|error| panic!("failed to read historical artifact {relative}: {error}"))
}
