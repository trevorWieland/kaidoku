use super::super::document::{ExtractionPage, FontDescriptor, RawElement};
use super::super::primitives::{
    BBox, BackendIdentifier, FontId, PageNumber, SchemaIdentifier, Sha256Digest, SourceRef,
    ValidationError,
};
use super::{ExtractionDocument, ExtractionSource};
use crate::{CharPayload, SpanPayload};

fn digest() -> Sha256Digest {
    Sha256Digest::new("0".repeat(64)).expect("valid sha256")
}

fn source() -> ExtractionSource {
    ExtractionSource::new(BackendIdentifier::Lopdf, digest(), 1024)
}

fn page(page_number: u32, elements: Vec<RawElement>) -> ExtractionPage {
    ExtractionPage::new(
        PageNumber::new(page_number).expect("valid"),
        400.0,
        600.0,
        elements,
    )
    .expect("valid page")
}

fn char_element(font_id: FontId) -> RawElement {
    RawElement::char(
        BBox::new(0.0, 0.0, 10.0, 10.0).expect("bbox"),
        SourceRef::new(1, 1, 0, 0, 0, 0).expect("source ref"),
        CharPayload::new("A", Some(font_id), 12.0, 0).expect("char payload"),
    )
}

fn span_element(font_id: FontId) -> RawElement {
    RawElement::span(
        BBox::new(0.0, 0.0, 10.0, 10.0).expect("bbox"),
        SourceRef::new(1, 1, 0, 0, 0, 1).expect("source ref"),
        SpanPayload::new("A", Some(font_id), 12.0).expect("span payload"),
    )
}

#[test]
fn extraction_document_rejects_empty_pages() {
    let error =
        ExtractionDocument::new(SchemaIdentifier::Phase1V2, source(), Vec::new(), Vec::new())
            .expect_err("empty pages should reject");
    assert!(matches!(error, ValidationError::EmptyPages));
}

#[test]
fn extraction_document_rejects_unsorted_or_duplicate_pages() {
    let pages = vec![page(2, Vec::new()), page(2, Vec::new())];
    let error = ExtractionDocument::new(SchemaIdentifier::Phase1V2, source(), Vec::new(), pages)
        .expect_err("duplicate page should reject");
    assert!(matches!(
        error,
        ValidationError::UnsortedOrDuplicatePage { page_number: 2 }
    ));

    let pages = vec![page(3, Vec::new()), page(1, Vec::new())];
    let error = ExtractionDocument::new(SchemaIdentifier::Phase1V2, source(), Vec::new(), pages)
        .expect_err("unsorted page should reject");
    assert!(matches!(
        error,
        ValidationError::UnsortedOrDuplicatePage { page_number: 1 }
    ));
}

#[test]
fn extraction_document_rejects_duplicate_font_ids() {
    let font_id = FontId::new(1).expect("font id");
    let fonts = vec![
        FontDescriptor {
            id: font_id,
            name: "A".to_string(),
        },
        FontDescriptor {
            id: font_id,
            name: "B".to_string(),
        },
    ];
    let error = ExtractionDocument::new(
        SchemaIdentifier::Phase1V2,
        source(),
        fonts,
        vec![page(1, Vec::new())],
    )
    .expect_err("duplicate font id should reject");
    assert!(matches!(
        error,
        ValidationError::DuplicateFontId { font_id: 1 }
    ));
}

#[test]
fn extraction_document_rejects_unknown_font_reference() {
    let font_id = FontId::new(7).expect("font id");
    let pages = vec![page(1, vec![char_element(font_id), span_element(font_id)])];
    let error = ExtractionDocument::new(SchemaIdentifier::Phase1V2, source(), Vec::new(), pages)
        .expect_err("unknown font reference should reject");
    assert!(matches!(
        error,
        ValidationError::UnknownFontReference {
            font_id: 7,
            page_number: 1
        }
    ));
}

#[test]
fn extraction_document_round_trips_through_json() {
    let font_id = FontId::new(1).expect("font id");
    let fonts = vec![FontDescriptor {
        id: font_id,
        name: "Helvetica".to_string(),
    }];
    let pages = vec![page(1, vec![char_element(font_id)])];
    let document = ExtractionDocument::new(SchemaIdentifier::Phase1V2, source(), fonts, pages)
        .expect("valid document");

    let json = serde_json::to_string(&document).expect("serialize");
    let round_tripped: ExtractionDocument = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(document, round_tripped);
}

#[test]
fn extraction_document_deserialization_rejects_invalid_json() {
    let raw = serde_json::json!({
        "schema_version": "kaidoku.phase1.v2",
        "source": {
            "backend": "lopdf",
            "input_sha256": "0".repeat(64),
            "input_bytes": 1024_u64
        },
        "fonts": [],
        "pages": []
    });
    let error = serde_json::from_value::<ExtractionDocument>(raw)
        .expect_err("empty pages should fail deserialize");
    assert!(error.to_string().contains("at least one page"));
}

#[test]
fn extraction_source_accessors_round_trip() {
    let original = ExtractionSource::new(BackendIdentifier::Lopdf, digest(), 42);
    assert_eq!(original.backend(), BackendIdentifier::Lopdf);
    assert_eq!(original.input_bytes(), 42);
    assert_eq!(original.input_sha256().as_str().len(), 64);

    let json = serde_json::to_string(&original).expect("serialize");
    let round_tripped: ExtractionSource = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(original, round_tripped);
}
