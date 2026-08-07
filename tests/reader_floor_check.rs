use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

const MKDOCS_CONFIG: &str = "docs_dir: docs\nexclude_docs: |\n  README.md\n";
const BARE_ADDRESS: &str = "https://basin-delineations-public.upstream.tech/grit/probe/\n";

fn run_probe(case_path: &str) -> Output {
    let probe_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("reader-floor-probes");
    fs::create_dir_all(&probe_root).unwrap();
    let repository = TempDir::new_in(probe_root).unwrap();
    let case_file = repository.path().join(case_path);
    fs::create_dir_all(case_file.parent().unwrap()).unwrap();
    fs::write(repository.path().join("mkdocs.yml"), MKDOCS_CONFIG).unwrap();
    fs::write(&case_file, BARE_ADDRESS).unwrap();

    run_git(repository.path(), &["init"]);
    run_git(repository.path(), &["add", "mkdocs.yml", case_path]);

    Command::new("python3")
        .arg(checker_path())
        .arg("--root")
        .arg(repository.path())
        .output()
        .unwrap()
}

fn run_git(repository: &Path, arguments: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(arguments)
        .status()
        .unwrap();
    assert!(status.success());
}

fn checker_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("scripts")
        .join("check_reader_floors.py")
}

fn assert_green(output: Output) {
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "reader-floor check passed: 0 bare occurrence(s)\n"
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn excludes_readme_below_test_fixture_and_golden_directories() {
    assert_green(run_probe(
        "crates/core/tests/fixtures/probe/goldens/case/README.md",
    ));
}

#[test]
fn excludes_dated_release_evidence() {
    assert_green(run_probe("docs/releases/2026-08-06-probe.md"));
}

#[test]
fn excludes_mkdocs_excluded_readme() {
    assert_green(run_probe("docs/README.md"));
}

#[test]
fn rejects_bare_address_on_published_quickstart() {
    let output = run_probe("docs/quickstart.md");
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        concat!(
            "reader-floor check failed: 1 bare occurrence(s) in 1 offering page(s)\n",
            "docs/quickstart.md:1: bare occurrence of basin-delineations-public\n",
        )
    );
    assert!(output.stderr.is_empty());
}
