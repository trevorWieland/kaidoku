//! Shared helpers for the Phase-1 integration test modules.
//!
//! Kept separate from [`phase1_tests`] so individual test files stay under the
//! workspace `check-lines` ceiling of 500 lines per source file.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) const REQUIRED_PHASE1_FIXTURES: [&str; 6] = [
    "doclaynet_simple_text.pdf",
    "doclaynet_multi_column.pdf",
    "doclaynet_mixed_content.pdf",
    "pdfjs_copy_paste_ligatures.pdf",
    "pdfjs_arabic_cid_true_type.pdf",
    "pdfjs_identity_to_unicode_map_char_code_of.pdf",
];

#[derive(Debug, serde::Deserialize)]
pub(crate) struct ProvenanceManifest {
    pub(crate) fixtures: Vec<ProvenanceFixture>,
}

#[derive(Debug, serde::Deserialize)]
pub(crate) struct ProvenanceFixture {
    pub(crate) name: String,
    pub(crate) sha256: String,
}

pub(crate) fn required_fixture_set() -> BTreeSet<String> {
    REQUIRED_PHASE1_FIXTURES
        .iter()
        .map(|name| (*name).to_string())
        .collect()
}

pub(crate) fn phase1_fixture_names() -> BTreeSet<String> {
    phase1_fixture_paths()
        .into_iter()
        .filter_map(|path| path.file_name().and_then(OsStr::to_str).map(str::to_string))
        .collect()
}

pub(crate) fn golden_fixture_names() -> BTreeSet<String> {
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

pub(crate) fn phase1_fixture_paths() -> Vec<PathBuf> {
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

pub(crate) fn load_manifest() -> ProvenanceManifest {
    let manifest_path = corpus_dir().join("provenance.json");
    let manifest_content =
        fs::read_to_string(&manifest_path).expect("provenance manifest is readable");
    serde_json::from_str(&manifest_content).expect("provenance manifest is valid json")
}

pub(crate) fn load_benchmark_baseline_fixture_names() -> BTreeSet<String> {
    let baseline_path = golden_dir().join("benchmarks.baseline.json");
    let content = fs::read_to_string(&baseline_path).expect("benchmark baseline is readable");
    let value: serde_json::Value =
        serde_json::from_str(&content).expect("benchmark baseline is valid json");

    if let Some(reports) = value.get("reports").and_then(serde_json::Value::as_array) {
        let mut names = BTreeSet::new();
        for report in reports {
            if let Some(fixtures) = report.get("fixtures").and_then(serde_json::Value::as_array) {
                for fixture in fixtures {
                    if let Some(name) = fixture.get("fixture").and_then(serde_json::Value::as_str) {
                        names.insert(name.to_string());
                    }
                }
            }
        }
        return names;
    }

    value
        .get("fixtures")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|fixture| {
            fixture
                .get("fixture")
                .and_then(serde_json::Value::as_str)
                .map(ToString::to_string)
        })
        .collect()
}

pub(crate) fn golden_path_for(fixture_path: &Path) -> PathBuf {
    let stem = fixture_path
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or("unknown");
    golden_dir().join(format!("{stem}.json"))
}

pub(crate) fn corpus_dir() -> PathBuf {
    workspace_root().join("tests/corpus/phase1")
}

pub(crate) fn golden_dir() -> PathBuf {
    workspace_root().join("tests/golden/phase1")
}

pub(crate) fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

pub(crate) fn bbox_contains(outer: crate::BBox, inner: crate::BBox, tolerance: f64) -> bool {
    let outer_left = outer.x() - tolerance;
    let outer_top = outer.y() - tolerance;
    let outer_right = outer.x() + outer.width() + tolerance;
    let outer_bottom = outer.y() + outer.height() + tolerance;

    let inner_left = inner.x();
    let inner_top = inner.y();
    let inner_right = inner.x() + inner.width();
    let inner_bottom = inner.y() + inner.height();

    inner_left >= outer_left
        && inner_top >= outer_top
        && inner_right <= outer_right
        && inner_bottom <= outer_bottom
}
