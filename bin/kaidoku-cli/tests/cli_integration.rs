use assert_cmd::Command;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/phase1")
        .join(name)
}

#[test]
fn extract_happy_path_matches_committed_goldens_exactly() {
    let fixtures = [
        "doclaynet_simple_text.pdf",
        "doclaynet_multi_column.pdf",
        "doclaynet_mixed_content.pdf",
    ];

    let output_dir = tempdir().expect("temp output dir");
    for fixture_name in fixtures {
        let fixture_path = fixture(fixture_name);
        Command::cargo_bin("kaidoku-cli")
            .expect("cli binary")
            .args([
                "extract",
                "--input",
                fixture_path.to_str().expect("fixture path"),
                "--output",
                output_dir.path().to_str().expect("output path"),
            ])
            .assert()
            .success();

        let output_file = output_dir.path().join(
            fixture_path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(|stem| format!("{stem}.json"))
                .expect("fixture stem"),
        );
        let golden_file = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/golden/phase1")
            .join(
                fixture_path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .map(|stem| format!("{stem}.json"))
                    .expect("golden stem"),
            );
        let actual = fs::read_to_string(output_file).expect("output json");
        let expected = fs::read_to_string(golden_file).expect("golden json");
        assert_eq!(
            actual, expected,
            "cli output drift for fixture {fixture_name}",
        );
    }
}

#[test]
fn extract_nonexistent_input_exits_nonzero() {
    let output_dir = tempdir().expect("temp output dir");

    Command::cargo_bin("kaidoku-cli")
        .expect("cli binary")
        .args([
            "extract",
            "--input",
            "/definitely/missing/input.pdf",
            "--output",
            output_dir.path().to_str().expect("output path"),
        ])
        .assert()
        .failure();
}

#[test]
fn extract_invalid_pages_spec_exits_nonzero() {
    let output_dir = tempdir().expect("temp output dir");
    let fixture_path = fixture("doclaynet_simple_text.pdf");

    Command::cargo_bin("kaidoku-cli")
        .expect("cli binary")
        .args([
            "extract",
            "--input",
            fixture_path.to_str().expect("fixture path"),
            "--output",
            output_dir.path().to_str().expect("output path"),
            "--pages",
            "0",
        ])
        .assert()
        .failure();
}

#[test]
fn extract_invalid_timeout_option_exits_nonzero() {
    let output_dir = tempdir().expect("temp output dir");
    let fixture_path = fixture("doclaynet_simple_text.pdf");

    Command::cargo_bin("kaidoku-cli")
        .expect("cli binary")
        .args([
            "extract",
            "--input",
            fixture_path.to_str().expect("fixture path"),
            "--output",
            output_dir.path().to_str().expect("output path"),
            "--max-wall-time-ms",
            "0",
        ])
        .assert()
        .failure();
}

#[test]
fn extract_duplicate_stems_use_deterministic_suffixes() {
    let work_dir = tempdir().expect("temp work dir");
    let output_dir = work_dir.path().join("out");
    fs::create_dir_all(&output_dir).expect("create output dir");

    let alpha = work_dir.path().join("alpha/input.pdf");
    let beta = work_dir.path().join("beta/input.pdf");
    fs::create_dir_all(alpha.parent().expect("alpha parent")).expect("alpha parent dir");
    fs::create_dir_all(beta.parent().expect("beta parent")).expect("beta parent dir");

    let fixture_bytes = fs::read(fixture("doclaynet_simple_text.pdf")).expect("fixture bytes");
    fs::write(&alpha, &fixture_bytes).expect("alpha fixture write");
    fs::write(&beta, &fixture_bytes).expect("beta fixture write");

    Command::cargo_bin("kaidoku-cli")
        .expect("cli binary")
        .args([
            "extract",
            "--input",
            alpha.to_str().expect("alpha path"),
            "--input",
            beta.to_str().expect("beta path"),
            "--output",
            output_dir.to_str().expect("output path"),
        ])
        .assert()
        .success();

    let first = output_dir.join("input_01.json");
    let second = output_dir.join("input_02.json");
    assert!(first.exists(), "missing first deterministic output");
    assert!(second.exists(), "missing second deterministic output");
}
