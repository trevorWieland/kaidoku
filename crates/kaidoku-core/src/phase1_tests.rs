use crate::{
    CancellationToken, ExtractError, ExtractOptions, ParseBackend, extract_pdf, to_canonical_json,
};
use pretty_assertions::assert_eq;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

const REQUIRED_PHASE1_FIXTURES: [&str; 6] = [
    "doclaynet_simple_text.pdf",
    "doclaynet_multi_column.pdf",
    "doclaynet_mixed_content.pdf",
    "pdfjs_copy_paste_ligatures.pdf",
    "pdfjs_arabic_cid_true_type.pdf",
    "pdfjs_identity_to_unicode_map_char_code_of.pdf",
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

    let bench_names = load_benchmark_baseline_fixture_names();
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
fn span_bboxes_cover_char_bboxes_per_operation() {
    for fixture_path in phase1_fixture_paths() {
        let bytes = fs::read(&fixture_path).expect("fixture must be readable");
        let document = extract_pdf(&bytes, ExtractOptions::default())
            .expect("fixture extraction should succeed");

        for page in document.pages {
            let mut grouped = BTreeMap::new();
            for element in page.elements {
                let source_ref = element.source_ref();
                let key = (source_ref.stream_index(), source_ref.operation_index());
                grouped.entry(key).or_insert_with(Vec::new).push(element);
            }

            for ((_stream, _operation), elements) in grouped {
                let spans = elements
                    .iter()
                    .filter_map(|element| element.span_payload().map(|_| element.bbox()))
                    .collect::<Vec<_>>();
                let chars = elements
                    .iter()
                    .filter_map(|element| element.char_payload().map(|_| element.bbox()))
                    .collect::<Vec<_>>();

                if spans.is_empty() || chars.is_empty() {
                    continue;
                }

                for char_bbox in chars {
                    let covered = spans
                        .iter()
                        .any(|span_bbox| bbox_contains(*span_bbox, char_bbox, 0.25));
                    assert!(
                        covered,
                        "char bbox was not covered by any span bbox on page {} in {}",
                        page.page_number.get(),
                        fixture_path.display(),
                    );
                }
            }
        }
    }
}

#[test]
fn bboxes_remain_within_page_bounds_with_tolerance() {
    for fixture_path in phase1_fixture_paths() {
        let bytes = fs::read(&fixture_path).expect("fixture must be readable");
        let document = extract_pdf(&bytes, ExtractOptions::default())
            .expect("fixture extraction should succeed");

        for page in document.pages {
            for element in page.elements {
                let bbox = element.bbox();
                assert!(
                    bbox.width() <= page.width * 20.0 && bbox.height() <= page.height * 20.0,
                    "bbox dimensions are implausibly large on page {} in {}",
                    page.page_number.get(),
                    fixture_path.display(),
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

fn load_benchmark_baseline_fixture_names() -> BTreeSet<String> {
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

fn bbox_contains(outer: crate::BBox, inner: crate::BBox, tolerance: f64) -> bool {
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
