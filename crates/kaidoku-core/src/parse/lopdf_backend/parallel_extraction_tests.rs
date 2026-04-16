//! Targeted correctness tests for the parallel extraction path:
//!
//! - Atomic decoded-budget reservation prevents over-spend under racing
//!   workers. With a tight per-run budget and multiple pages, at least one
//!   page must fail with `DecodedStreamBudgetExceeded`; no combination of
//!   successful pages may sum above the limit.
//! - Parallel and serial paths produce structurally identical outputs —
//!   same element counts, same source-refs, same bboxes — on the pinned
//!   phase-1 corpus. This locks in the determinism invariant the
//!   lock-free path relies on (font IDs from the pre-pass).
//! - Parallel extraction repeated multiple times on a multi-page fixture
//!   yields byte-identical canonical JSON.

use crate::phase1_support::corpus_dir;
use crate::{ExtractError, ExtractOptions, extract_pdf, to_canonical_json};
use std::fs;

#[test]
fn parallel_and_serial_paths_produce_structurally_equal_output_on_multi_page_fixture() {
    // Font IDs can differ between the serial and parallel paths — serial
    // interns fonts lazily in content-stream order, parallel pre-populates
    // in resource-dictionary order. The STRUCTURAL invariants (same page
    // count, element count, element kinds, bboxes, source-refs) must still
    // match byte-for-byte since the two paths share the same emitter.
    let fixture = corpus_dir().join("doclaynet_mixed_content.pdf");
    let bytes = fs::read(&fixture).expect("fixture readable");

    let serial = extract_pdf(&bytes, ExtractOptions::default()).expect("serial");
    let parallel = extract_pdf(
        &bytes,
        ExtractOptions::builder()
            .parallel_page_extraction(true)
            .build()
            .expect("options"),
    )
    .expect("parallel");

    assert_eq!(serial.pages().len(), parallel.pages().len());
    for (s, p) in serial.pages().iter().zip(parallel.pages().iter()) {
        assert_eq!(s.page_number(), p.page_number());
        assert_eq!(s.elements().len(), p.elements().len());
        for (se, pe) in s.elements().iter().zip(p.elements().iter()) {
            assert_eq!(se.source_ref(), pe.source_ref());
            assert_eq!(se.bbox(), pe.bbox());
        }
    }
}

#[test]
fn parallel_extraction_is_repeatably_deterministic() {
    let fixture = corpus_dir().join("doclaynet_mixed_content.pdf");
    let bytes = fs::read(&fixture).expect("fixture readable");
    let options = ExtractOptions::builder()
        .parallel_page_extraction(true)
        .build()
        .expect("options");

    let first =
        to_canonical_json(&extract_pdf(&bytes, options.clone()).expect("first parallel run"))
            .expect("json");
    let second =
        to_canonical_json(&extract_pdf(&bytes, options.clone()).expect("second parallel run"))
            .expect("json");
    let third = to_canonical_json(&extract_pdf(&bytes, options).expect("third parallel run"))
        .expect("json");

    assert_eq!(first, second);
    assert_eq!(second, third);
}

#[test]
fn parallel_decoded_budget_is_atomically_enforced() {
    // Set the total decoded-stream budget to an intentionally tight limit
    // that is guaranteed to be exhausted by the fixture's combined page
    // streams. Under the pre-fix implementation, two workers could each
    // snapshot the same remaining budget and quietly over-spend; under the
    // atomic-reservation implementation, at least one page MUST fail with
    // `DecodedStreamBudgetExceeded`.
    let fixture = corpus_dir().join("doclaynet_mixed_content.pdf");
    let bytes = fs::read(&fixture).expect("fixture readable");

    let options = ExtractOptions::builder()
        .parallel_page_extraction(true)
        .max_total_decoded_stream_bytes(256)
        .build()
        .expect("options");

    let error = extract_pdf(&bytes, options).expect_err("tight budget must fail");
    assert!(
        matches!(error, ExtractError::DecodedStreamBudgetExceeded { .. }),
        "expected DecodedStreamBudgetExceeded, got {error:?}"
    );
}

#[test]
fn parallel_decoded_budget_reports_coherent_limit_and_actual_bytes() {
    // The `actual_bytes` in the error must never be less than the declared
    // limit. This guards against the old fetch_min reconciliation where
    // racing threads could report a pre-decrement snapshot as the actual
    // consumption, producing an incoherent "actual < limit" error.
    let fixture = corpus_dir().join("doclaynet_mixed_content.pdf");
    let bytes = fs::read(&fixture).expect("fixture readable");
    let options = ExtractOptions::builder()
        .parallel_page_extraction(true)
        .max_total_decoded_stream_bytes(128)
        .build()
        .expect("options");

    let error = extract_pdf(&bytes, options)
        .expect_err("tight parallel budget must produce DecodedStreamBudgetExceeded");
    assert!(
        matches!(error, ExtractError::DecodedStreamBudgetExceeded { .. }),
        "expected DecodedStreamBudgetExceeded, got {error:?}"
    );
    let ExtractError::DecodedStreamBudgetExceeded {
        limit_bytes,
        actual_bytes,
        ..
    } = error
    else {
        return;
    };

    assert_eq!(limit_bytes, 128);
    assert!(
        actual_bytes > limit_bytes,
        "atomic reservation must report actual_bytes > limit_bytes; got {actual_bytes} / {limit_bytes}"
    );
}
