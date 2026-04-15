use super::{ExtractionLimits, extract_page_elements};
use crate::{PageNumber, RawPayload};
use lopdf::{Document, Object, ObjectId, Stream, dictionary};
use std::collections::{BTreeSet, HashMap};

#[test]
fn tj_elements_have_unique_source_ref_indices() {
    let (document, page_id) = page_document_with_streams(&[b"BT 10 20 Td [(AB) 120 (CD)] TJ ET"]);
    let page_number = PageNumber::new(1);
    assert!(page_number.is_ok());
    let Ok(page_number) = page_number else { return };

    let elements = extract_page_elements(
        &document,
        page_number,
        page_id,
        3,
        &HashMap::new(),
        ExtractionLimits {
            operation_budget: 10_000,
            max_elements: 10_000,
            stream_byte_limit: 1_000_000,
        },
    );
    assert!(elements.is_ok());
    let Ok(elements) = elements else { return };

    let mut op_indices = BTreeSet::new();
    for element in elements {
        if element.source_ref().operation_index() == 2 {
            assert!(
                op_indices.insert(element.source_ref().element_index()),
                "duplicate element index for TJ operation"
            );
        }
    }
    assert!(
        op_indices.len() >= 5,
        "expected span + chars for TJ operation"
    );
}

#[test]
fn text_state_persists_across_stream_boundaries() {
    let (document, page_id) = page_document_with_streams(&[b"BT 10 20 Td", b"(A) Tj ET"]);
    let page_number = PageNumber::new(1);
    assert!(page_number.is_ok());
    let Ok(page_number) = page_number else { return };

    let elements = extract_page_elements(
        &document,
        page_number,
        page_id,
        3,
        &HashMap::new(),
        ExtractionLimits {
            operation_budget: 10_000,
            max_elements: 10_000,
            stream_byte_limit: 1_000_000,
        },
    );
    assert!(elements.is_ok());
    let Ok(elements) = elements else { return };

    let span = elements.iter().find(|element| match element.payload() {
        RawPayload::Span(payload) => payload.text == "A",
        _ => false,
    });
    assert!(span.is_some());
    let Some(span) = span else { return };

    assert!(
        span.bbox().x() >= 9.0,
        "expected carried text position from prior stream, got x={}",
        span.bbox().x()
    );
}

fn page_document_with_streams(streams: &[&[u8]]) -> (Document, ObjectId) {
    let mut document = Document::new();
    let page_id: ObjectId = (1, 0);

    let mut content_refs = Vec::new();
    for (index, stream_bytes) in streams.iter().enumerate() {
        let stream_id_number = u32::try_from(index + 2);
        assert!(stream_id_number.is_ok());
        let Ok(stream_id_number) = stream_id_number else {
            continue;
        };
        let stream_id = (stream_id_number, 0);
        let stream = Stream::new(dictionary! {}, (*stream_bytes).to_vec());
        document.objects.insert(stream_id, Object::Stream(stream));
        content_refs.push(Object::Reference(stream_id));
    }

    document.objects.insert(
        page_id,
        Object::Dictionary(dictionary! {
            "Type" => "Page",
            "Contents" => Object::Array(content_refs),
        }),
    );

    (document, page_id)
}
