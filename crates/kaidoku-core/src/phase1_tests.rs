use crate::phase1_support::{
    REQUIRED_PHASE1_FIXTURES, bbox_contains, corpus_dir, golden_fixture_names, golden_path_for,
    load_benchmark_baseline_fixture_names, load_manifest, phase1_fixture_names,
    phase1_fixture_paths, required_fixture_set,
};
use crate::{
    CancellationToken, ExtractError, ExtractOptions, ParseBackend, extract_pdf, to_canonical_json,
};
use pretty_assertions::assert_eq;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;

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
            for element in page.elements() {
                let source_ref = element.source_ref();
                let key = (
                    source_ref.stream_index(),
                    source_ref.operation_index(),
                    source_ref.element_index(),
                );
                assert!(
                    keys.insert(key),
                    "duplicate source_ref within page {} in fixture {}",
                    page.page_number().get(),
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
            for element in page.elements() {
                let text = if let Some(span) = element.span_payload() {
                    span.text()
                } else if let Some(character) = element.char_payload() {
                    character.text()
                } else {
                    continue;
                };

                let has_bad_controls = text.chars().any(|character| {
                    character.is_control() && !matches!(character, '\n' | '\r' | '\t')
                });
                assert!(
                    !has_bad_controls,
                    "decoded text contains control characters on page {} in {}: {:?}",
                    page.page_number().get(),
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
            for element in page.elements() {
                let source_ref = element.source_ref();
                let key = (source_ref.stream_index(), source_ref.operation_index());
                grouped
                    .entry(key)
                    .or_insert_with(Vec::new)
                    .push(element.clone());
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
                        page.page_number().get(),
                        fixture_path.display(),
                    );
                }
            }
        }
    }
}

#[test]
fn bboxes_remain_within_page_bounds_with_tolerance() {
    // Tight bound: production PDFs in the corpus have bboxes that sit inside
    // a 1.5× page-dimension envelope. The historical 20× bound masked severe
    // regressions (see Phase-1 audit P2). If a legitimate fixture exceeds this,
    // introduce a per-element whitelist rather than relaxing the bound.
    const MAX_DIM_MULTIPLIER: f64 = 1.5;

    for fixture_path in phase1_fixture_paths() {
        let bytes = fs::read(&fixture_path).expect("fixture must be readable");
        let document = extract_pdf(&bytes, ExtractOptions::default())
            .expect("fixture extraction should succeed");

        for page in document.pages {
            for element in page.elements() {
                let bbox = element.bbox();
                assert!(
                    bbox.width().is_finite()
                        && bbox.height().is_finite()
                        && bbox.width() >= 0.0
                        && bbox.height() >= 0.0,
                    "bbox has non-finite or negative dimensions on page {} in {}: {:?}",
                    page.page_number().get(),
                    fixture_path.display(),
                    bbox,
                );
                assert!(
                    bbox.width() <= page.width() * MAX_DIM_MULTIPLIER
                        && bbox.height() <= page.height() * MAX_DIM_MULTIPLIER,
                    "bbox {:?} exceeds {}× page bounds ({} × {}) on page {} in {}",
                    bbox,
                    MAX_DIM_MULTIPLIER,
                    page.width(),
                    page.height(),
                    page.page_number().get(),
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
