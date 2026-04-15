use crate::{
    CancellationToken, ExtractError, ExtractOptions, ParseBackend, extract_pdf, to_canonical_json,
};
use pretty_assertions::assert_eq;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

const REQUIRED_PHASE1_FIXTURES: [&str; 3] = [
    "doclaynet_simple_text.pdf",
    "doclaynet_multi_column.pdf",
    "doclaynet_mixed_content.pdf",
];

#[derive(Debug, serde::Deserialize)]
struct ProvenanceManifest {
    fixtures: Vec<ProvenanceFixture>,
}

#[derive(Debug, serde::Deserialize)]
struct ProvenanceFixture {
    name: String,
    sha256: String,
}

#[derive(Debug, serde::Deserialize)]
struct BenchBaseline {
    fixtures: Vec<BenchFixture>,
}

#[derive(Debug, serde::Deserialize)]
struct BenchFixture {
    fixture: String,
}

#[test]
fn golden_outputs_match_phase1_fixtures() {
    let fixture_paths = phase1_fixture_paths();

    for fixture_path in fixture_paths {
        let bytes = fs::read(&fixture_path);
        assert!(
            bytes.is_ok(),
            "failed reading fixture: {}",
            fixture_path.display()
        );

        let Ok(bytes) = bytes else { return };

        let result = extract_pdf(&bytes, ExtractOptions::default());
        assert!(
            result.is_ok(),
            "extraction failed for fixture {}: {result:?}",
            fixture_path.display(),
        );

        let Ok(document) = result else { return };

        let json = to_canonical_json(&document);
        assert!(
            json.is_ok(),
            "serialization failed for fixture {}",
            fixture_path.display()
        );

        let Ok(json) = json else { return };
        let golden_path = golden_path_for(&fixture_path);
        let golden = fs::read_to_string(&golden_path);
        assert!(
            golden.is_ok(),
            "missing golden file: {}",
            golden_path.display()
        );

        let Ok(golden) = golden else { return };
        assert_eq!(
            golden,
            json,
            "golden mismatch for fixture {}",
            fixture_path.display()
        );
    }
}

#[test]
fn extraction_is_deterministic_for_same_input() {
    let fixture_paths = phase1_fixture_paths();

    for fixture_path in fixture_paths {
        let bytes = fs::read(&fixture_path);
        assert!(
            bytes.is_ok(),
            "failed reading fixture: {}",
            fixture_path.display()
        );

        let Ok(bytes) = bytes else { return };

        let first = extract_pdf(&bytes, ExtractOptions::default());
        let second = extract_pdf(&bytes, ExtractOptions::default());
        assert!(
            first.is_ok(),
            "first extraction failed for {}",
            fixture_path.display()
        );
        assert!(
            second.is_ok(),
            "second extraction failed for {}",
            fixture_path.display()
        );

        let (Ok(first), Ok(second)) = (first, second) else {
            return;
        };

        let first_json = to_canonical_json(&first);
        let second_json = to_canonical_json(&second);
        assert!(first_json.is_ok());
        assert!(second_json.is_ok());

        let (Ok(first_json), Ok(second_json)) = (first_json, second_json) else {
            return;
        };

        assert_eq!(
            first_json,
            second_json,
            "nondeterministic extraction for {}",
            fixture_path.display(),
        );
    }
}

#[test]
fn fixture_sets_are_pinned_and_in_parity() {
    let fixture_names = phase1_fixture_names();
    let expected = required_fixture_set();

    assert_eq!(
        fixture_names, expected,
        "phase1 fixture corpus must exactly match required fixture names"
    );

    let manifest = load_manifest();
    let manifest_names = manifest
        .fixtures
        .iter()
        .map(|fixture| fixture.name.clone())
        .collect::<BTreeSet<String>>();
    assert_eq!(
        manifest_names, expected,
        "manifest fixture names must exactly match required fixture set"
    );

    let golden_names = golden_fixture_names();
    let expected_golden = expected
        .iter()
        .map(|name| name.trim_end_matches(".pdf").to_string())
        .collect::<BTreeSet<String>>();
    assert_eq!(
        golden_names, expected_golden,
        "golden fixture names must exactly match corpus fixture set"
    );

    let bench = load_benchmark_baseline();
    let bench_names = bench
        .fixtures
        .iter()
        .map(|fixture| fixture.fixture.clone())
        .collect::<BTreeSet<String>>();
    assert_eq!(
        bench_names, expected,
        "benchmark baseline fixture set must exactly match required fixture set"
    );
}

#[test]
fn fixture_provenance_hashes_match_files() {
    let manifest = load_manifest();

    assert_eq!(
        manifest.fixtures.len(),
        REQUIRED_PHASE1_FIXTURES.len(),
        "provenance fixture count must match required fixture count"
    );

    for fixture in manifest.fixtures {
        let path = corpus_dir().join(&fixture.name);
        let bytes = fs::read(&path);
        assert!(
            bytes.is_ok(),
            "missing fixture listed in provenance: {}",
            path.display()
        );

        let Ok(bytes) = bytes else { return };
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let digest = hasher.finalize();
        let actual = format!("{digest:x}");
        assert_eq!(
            actual, fixture.sha256,
            "fixture hash mismatch for {}",
            fixture.name
        );
    }
}

#[test]
fn source_refs_are_unique_per_operation() {
    for fixture_path in phase1_fixture_paths() {
        let bytes = fs::read(&fixture_path).expect("fixture must be readable");
        let document = extract_pdf(&bytes, ExtractOptions::default())
            .expect("fixture extraction should succeed");

        for page in document.pages {
            let mut keys = BTreeSet::new();
            for element in page.elements {
                let source_ref = element.source_ref();
                let key = (
                    source_ref.stream_index(),
                    source_ref.operation_index(),
                    source_ref.element_index(),
                );
                assert!(
                    keys.insert(key),
                    "duplicate source_ref within page {} in fixture {}",
                    page.page_number.get(),
                    fixture_path.display(),
                );
            }
        }
    }
}

#[test]
fn decoded_text_avoids_control_character_gibberish() {
    for fixture_path in phase1_fixture_paths() {
        let bytes = fs::read(&fixture_path).expect("fixture must be readable");
        let document = extract_pdf(&bytes, ExtractOptions::default())
            .expect("fixture extraction should succeed");

        for page in document.pages {
            for element in page.elements {
                let text = if let Some(span) = element.span_payload() {
                    span.text.as_str()
                } else if let Some(character) = element.char_payload() {
                    character.text.as_str()
                } else {
                    continue;
                };

                let has_bad_controls = text.chars().any(|character| {
                    character.is_control() && !matches!(character, '\n' | '\r' | '\t')
                });
                assert!(
                    !has_bad_controls,
                    "decoded text contains control characters on page {} in {}: {:?}",
                    page.page_number.get(),
                    fixture_path.display(),
                    text,
                );
            }
        }
    }
}

#[test]
fn input_size_limit_rejects_oversized_payloads() {
    let options = ExtractOptions::builder()
        .max_input_bytes(1)
        .build()
        .expect("options");

    let err = extract_pdf(&[1_u8, 2_u8], options).expect_err("oversized payload must fail early");
    assert!(matches!(
        err,
        ExtractError::InputTooLarge {
            limit_bytes: 1,
            actual_bytes: 2,
        }
    ));
}

#[test]
fn explicit_lopdf_backend_selection_is_supported() {
    let fixture_path = corpus_dir().join("doclaynet_simple_text.pdf");
    let bytes = fs::read(&fixture_path).expect("fixture must be readable");
    let options = ExtractOptions::builder()
        .backend(ParseBackend::Lopdf)
        .build()
        .expect("options");

    let document =
        extract_pdf(&bytes, options).expect("explicit backend extraction should succeed");
    assert!(!document.pages.is_empty());
}

#[test]
fn cooperative_cancellation_token_short_circuits_extraction() {
    let fixture_path = corpus_dir().join("doclaynet_simple_text.pdf");
    let bytes = fs::read(&fixture_path).expect("fixture must be readable");
    let token = CancellationToken::new();
    token.cancel();
    let options = ExtractOptions::builder()
        .cancellation_token(token)
        .build()
        .expect("options");

    let error = extract_pdf(&bytes, options).expect_err("cancelled extraction must fail");
    assert!(matches!(error, ExtractError::ExtractionCancelled { .. }));
}

fn required_fixture_set() -> BTreeSet<String> {
    REQUIRED_PHASE1_FIXTURES
        .iter()
        .map(|name| (*name).to_string())
        .collect()
}

fn phase1_fixture_names() -> BTreeSet<String> {
    phase1_fixture_paths()
        .into_iter()
        .filter_map(|path| path.file_name().and_then(OsStr::to_str).map(str::to_string))
        .collect()
}

fn golden_fixture_names() -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    if let Ok(entries) = fs::read_dir(golden_dir()) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension() == Some(OsStr::new("json"))
                && path.file_name() != Some(OsStr::new("benchmarks.baseline.json"))
                && path.file_name() != Some(OsStr::new("benchmarks.current.json"))
                && let Some(stem) = path.file_stem().and_then(OsStr::to_str)
            {
                names.insert(stem.to_string());
            }
        }
    }

    names
}

fn phase1_fixture_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(entries) = fs::read_dir(corpus_dir()) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension() == Some(OsStr::new("pdf")) {
                paths.push(path);
            }
        }
    }
    paths.sort();
    paths
}

fn load_manifest() -> ProvenanceManifest {
    let manifest_path = corpus_dir().join("provenance.json");
    let manifest_content =
        fs::read_to_string(&manifest_path).expect("provenance manifest is readable");
    serde_json::from_str(&manifest_content).expect("provenance manifest is valid json")
}

fn load_benchmark_baseline() -> BenchBaseline {
    let baseline_path = golden_dir().join("benchmarks.baseline.json");
    let content = fs::read_to_string(&baseline_path).expect("benchmark baseline is readable");
    serde_json::from_str(&content).expect("benchmark baseline is valid json")
}

fn golden_path_for(fixture_path: &Path) -> PathBuf {
    let stem = fixture_path
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or("unknown");
    golden_dir().join(format!("{stem}.json"))
}

fn corpus_dir() -> PathBuf {
    workspace_root().join("tests/corpus/phase1")
}

fn golden_dir() -> PathBuf {
    workspace_root().join("tests/golden/phase1")
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
}
