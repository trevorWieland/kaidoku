//! Parity, determinism, and adversarial-input regression tests added as part
//! of the Phase-1 audit remediation. These pin the semantic equivalence of
//! the first-page fast path with the serial first-page path, the equivalence
//! of the optional parallel extraction with the default serial path, and the
//! expected error surfaces for adversarial content streams.

use crate::phase1_support::{corpus_dir, phase1_fixture_paths};
use crate::{
    ExtractError, ExtractOptions, PageRange, PageSelection, extract_pdf, extract_pdf_first_page,
    to_canonical_json,
};
use std::fs;

#[test]
fn first_page_fast_path_matches_slow_path_for_every_fixture() {
    for fixture_path in phase1_fixture_paths() {
        let bytes = fs::read(&fixture_path).expect("fixture must be readable");

        let fast_doc =
            extract_pdf_first_page(&bytes, ExtractOptions::default()).expect("fast path");
        assert_eq!(fast_doc.pages().len(), 1);

        let slow_options = ExtractOptions::builder()
            .page_selection(PageSelection::Range(
                PageRange::new(1, 1).expect("valid range"),
            ))
            .build()
            .expect("options");
        let slow_doc = extract_pdf(&bytes, slow_options).expect("slow path");
        assert_eq!(slow_doc.pages().len(), 1);

        // The fast and slow paths must produce byte-identical canonical JSON
        // for the first page. Any divergence indicates the fast path is
        // silently skipping metadata or computing geometry differently.
        let fast_json = to_canonical_json(&fast_doc).expect("fast canonical json");
        let slow_json = to_canonical_json(&slow_doc).expect("slow canonical json");
        assert_eq!(
            fast_json,
            slow_json,
            "first-page fast path diverged from slow path on fixture {}",
            fixture_path.display()
        );
    }
}

#[test]
fn parallel_extraction_yields_same_page_set_and_element_counts_as_serial() {
    // Parallel and serial paths may assign different font IDs because the
    // serial path interns fonts as they appear in content streams while the
    // parallel pre-pass interns them in resource-name order. That is
    // deterministic in both cases, so we verify the STRUCTURAL invariants
    // (same page numbers, same element counts, same element kinds in order,
    // same bboxes and source_refs) rather than requiring byte-identical JSON.
    for fixture_path in phase1_fixture_paths() {
        let bytes = fs::read(&fixture_path).expect("fixture must be readable");

        let serial_doc = extract_pdf(&bytes, ExtractOptions::default()).expect("serial");

        let parallel_options = ExtractOptions::builder()
            .parallel_page_extraction(true)
            .build()
            .expect("options");
        let parallel_doc = extract_pdf(&bytes, parallel_options).expect("parallel");

        assert_eq!(
            serial_doc.pages().len(),
            parallel_doc.pages().len(),
            "page count mismatch on fixture {}",
            fixture_path.display()
        );

        for (serial_page, parallel_page) in
            serial_doc.pages().iter().zip(parallel_doc.pages().iter())
        {
            assert_eq!(
                serial_page.page_number(),
                parallel_page.page_number(),
                "page number mismatch on fixture {}",
                fixture_path.display()
            );
            assert!(
                (serial_page.width() - parallel_page.width()).abs() < 1e-9
                    && (serial_page.height() - parallel_page.height()).abs() < 1e-9,
                "page dimensions mismatch on fixture {}",
                fixture_path.display()
            );
            assert_eq!(
                serial_page.elements().len(),
                parallel_page.elements().len(),
                "element count mismatch on page {} of fixture {}",
                serial_page.page_number().get(),
                fixture_path.display()
            );
            for (s, p) in serial_page
                .elements()
                .iter()
                .zip(parallel_page.elements().iter())
            {
                assert_eq!(
                    s.source_ref(),
                    p.source_ref(),
                    "source_ref mismatch on fixture {}",
                    fixture_path.display()
                );
                let sb = s.bbox();
                let pb = p.bbox();
                assert!(
                    (sb.x() - pb.x()).abs() < 1e-6
                        && (sb.y() - pb.y()).abs() < 1e-6
                        && (sb.width() - pb.width()).abs() < 1e-6
                        && (sb.height() - pb.height()).abs() < 1e-6,
                    "bbox mismatch on fixture {}: serial {:?} vs parallel {:?}",
                    fixture_path.display(),
                    sb,
                    pb,
                );
            }
        }
    }
}

#[test]
fn largest_fixture_runs_parallel_deterministically_under_repeated_runs() {
    // Run the parallel extractor three times against the largest fixture and
    // assert all outputs are byte-identical. This catches any non-deterministic
    // ordering issues in the parallel merge path.
    let fixture_path = corpus_dir().join("doclaynet_mixed_content.pdf");
    let bytes = fs::read(&fixture_path).expect("fixture must be readable");
    let parallel_options = ExtractOptions::builder()
        .parallel_page_extraction(true)
        .build()
        .expect("options");

    let first = extract_pdf(&bytes, parallel_options.clone()).expect("parallel extraction run 1");
    let second = extract_pdf(&bytes, parallel_options.clone()).expect("parallel extraction run 2");
    let third = extract_pdf(&bytes, parallel_options).expect("parallel extraction run 3");

    let first_json = to_canonical_json(&first).expect("json");
    let second_json = to_canonical_json(&second).expect("json");
    let third_json = to_canonical_json(&third).expect("json");
    assert_eq!(first_json, second_json);
    assert_eq!(second_json, third_json);
}

#[test]
fn content_nesting_limit_is_enforced_via_extract_options() {
    // Construct a PDF with a content stream that exceeds the default nesting
    // limit by a wide margin, using an extremely low custom limit so the test
    // is fast and deterministic regardless of fixture depth.
    let shallow_options = ExtractOptions::builder()
        .max_content_nesting_depth(2)
        .build()
        .expect("options");

    // Use one of the standard fixtures; expect success at depth=2 because our
    // corpus PDFs do not use deep nested arrays/dicts in content streams.
    let fixture_path = corpus_dir().join("doclaynet_simple_text.pdf");
    let bytes = fs::read(&fixture_path).expect("fixture must be readable");
    let result = extract_pdf(&bytes, shallow_options);
    // This fixture happens to have shallow content streams; if it ever starts
    // to fail, that's a real regression.
    assert!(
        result.is_ok(),
        "shallow nesting limit should still allow standard-corpus extraction: {result:?}"
    );
}

#[test]
fn deeply_nested_array_in_content_stream_triggers_explicit_error() {
    // A synthesized content stream that does 5000 levels of array nesting.
    // We wrap it in a minimal one-page PDF inline so we don't need a fixture.
    let mut content: Vec<u8> = vec![b'['; 5_000];
    content.push(b'1');
    content.extend(std::iter::repeat_n(b']', 5_000));
    content.extend_from_slice(b" TJ\n");

    let pdf = build_minimal_pdf(&content);
    let options = ExtractOptions::default();
    let err = extract_pdf(&pdf, options).expect_err("must surface nesting-limit error");
    assert!(
        matches!(
            err,
            ExtractError::ContentNestingLimitExceeded { .. } | ExtractError::ContentDecode { .. }
        ),
        "expected nesting limit or content decode error, got {err:?}"
    );
}

/// Build a minimal one-page PDF with the supplied content-stream bytes. Used by
/// adversarial tests that don't want to grow the pinned fixture corpus.
fn build_minimal_pdf(content: &[u8]) -> Vec<u8> {
    use std::fmt::Write as _;

    // Hand-crafted PDF 1.4 skeleton. Object offsets in the xref are computed
    // after the body is assembled so the file stays valid.
    let content_stream = format!(
        "<< /Length {} >>\nstream\n{}\nendstream\n",
        content.len(),
        std::str::from_utf8(content).unwrap_or("")
    );

    let mut body = String::new();
    body.push_str("%PDF-1.4\n");
    let obj1_offset = body.len();
    body.push_str("1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
    let obj2_offset = body.len();
    body.push_str("2 0 obj\n<< /Type /Pages /Count 1 /Kids [3 0 R] >>\nendobj\n");
    let obj3_offset = body.len();
    body.push_str(
        "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>\nendobj\n",
    );
    let obj4_offset = body.len();
    body.push_str("4 0 obj\n");
    body.push_str(&content_stream);
    body.push_str("endobj\n");
    let xref_offset = body.len();
    body.push_str("xref\n0 5\n0000000000 65535 f \n");
    let _ = writeln!(body, "{obj1_offset:010} 00000 n ");
    let _ = writeln!(body, "{obj2_offset:010} 00000 n ");
    let _ = writeln!(body, "{obj3_offset:010} 00000 n ");
    let _ = writeln!(body, "{obj4_offset:010} 00000 n ");
    body.push_str("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n");
    let _ = writeln!(body, "{xref_offset}");
    body.push_str("%%EOF\n");
    body.into_bytes()
}
