use crate::{ExtractOptions, extract_pdf, to_canonical_json};
use pretty_assertions::assert_eq;
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, serde::Deserialize)]
struct ProvenanceManifest {
    fixtures: Vec<ProvenanceFixture>,
}

#[derive(Debug, serde::Deserialize)]
struct ProvenanceFixture {
    name: String,
    sha256: String,
}

#[test]
fn golden_outputs_match_phase1_fixtures() {
    let fixture_paths = phase1_fixture_paths();
    assert!(!fixture_paths.is_empty(), "phase1 fixtures are missing");

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
    assert!(!fixture_paths.is_empty(), "phase1 fixtures are missing");

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
fn fixture_provenance_hashes_match_files() {
    let manifest_path = corpus_dir().join("provenance.json");
    let manifest_content = fs::read_to_string(&manifest_path);
    assert!(
        manifest_content.is_ok(),
        "failed reading provenance manifest {}",
        manifest_path.display(),
    );

    let Ok(manifest_content) = manifest_content else {
        return;
    };

    let manifest: Result<ProvenanceManifest, _> = serde_json::from_str(&manifest_content);
    assert!(
        manifest.is_ok(),
        "failed parsing provenance manifest {}",
        manifest_path.display(),
    );

    let Ok(manifest) = manifest else { return };
    assert!(
        !manifest.fixtures.is_empty(),
        "provenance fixtures are empty"
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
