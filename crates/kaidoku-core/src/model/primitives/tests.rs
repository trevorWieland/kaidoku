use super::{
    BBox, BackendIdentifier, CharPayload, FontDescriptor, FontId, ImagePayload, RawElement,
    SchemaIdentifier, Sha256Digest, SourceRef, SpanPayload, ValidationError,
};
use serde_json::from_str;

#[test]
fn bbox_rejects_invalid_values() {
    let invalid_width = BBox::new(0.0, 0.0, -1.0, 1.0);
    assert_eq!(invalid_width, Err(ValidationError::NegativeDimension));

    let invalid_nan = BBox::new(f64::NAN, 0.0, 1.0, 1.0);
    assert_eq!(invalid_nan, Err(ValidationError::NonFiniteCoordinate));
}

#[test]
fn source_ref_stable_key_is_deterministic() {
    let source_ref = SourceRef::new(1, 12, 0, 0, 9, 3);
    assert!(source_ref.is_ok());

    let Ok(source_ref) = source_ref else { return };
    assert_eq!(source_ref.stable_key(), "p1-po12g0-o9-s0-i3");
}

#[test]
fn page_number_and_source_ref_deserialize_paths_are_validated() {
    let invalid_page = SourceRef::new(0, 1, 0, 0, 0, 0);
    assert_eq!(invalid_page, Err(ValidationError::InvalidPageNumber));

    let parsed = from_str::<SourceRef>(
        r#"{
                "page_number": 3,
                "page_object_number": 12,
                "page_object_generation": 0,
                "stream_index": 2,
                "operation_index": 7,
                "element_index": 4
            }"#,
    );
    assert!(parsed.is_ok());

    let Ok(parsed) = parsed else { return };
    assert_eq!(parsed.page_number().get(), 3);
    assert_eq!(parsed.page_object_number(), 12);
}

#[test]
fn typed_identifiers_round_trip() {
    let schema = from_str::<SchemaIdentifier>(r#""kaidoku.phase1.v2""#);
    assert!(matches!(schema, Ok(SchemaIdentifier::Phase1V2)));

    let backend = from_str::<BackendIdentifier>(r#""lopdf""#);
    assert!(matches!(backend, Ok(BackendIdentifier::Lopdf)));

    let digest =
        Sha256Digest::new("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef");
    assert!(digest.is_ok());

    let invalid_digest = Sha256Digest::new("abc");
    assert_eq!(invalid_digest, Err(ValidationError::InvalidSha256Digest));
}

#[test]
fn raw_element_variant_accessors_are_type_safe() {
    let bbox = BBox::new(0.0, 1.0, 2.0, 3.0);
    assert!(bbox.is_ok());
    let Ok(bbox) = bbox else { return };

    let source_ref = SourceRef::new(1, 9, 0, 2, 3, 4);
    assert!(source_ref.is_ok());
    let Ok(source_ref) = source_ref else { return };

    let font_id = FontId::new(1).expect("font id");
    let char_element = RawElement::char(
        bbox,
        source_ref,
        CharPayload {
            text: "A".to_string(),
            font_id: Some(font_id),
            font_size: 12.0,
            char_index: 0,
        },
    );
    assert!(char_element.char_payload().is_some());
    assert!(char_element.span_payload().is_none());
    assert!(char_element.image_payload().is_none());

    let span_element = RawElement::span(
        bbox,
        source_ref,
        SpanPayload {
            text: "AB".to_string(),
            font_id: None,
            font_size: 10.0,
        },
    );
    assert!(span_element.char_payload().is_none());
    assert!(span_element.span_payload().is_some());
    assert!(span_element.image_payload().is_none());

    let image_element = RawElement::image(
        bbox,
        source_ref,
        ImagePayload {
            name: "Im1".to_string(),
            width_px: 16,
            height_px: 8,
            color_space: Some("DeviceRGB".to_string()),
            bits_per_component: Some(8),
        },
    );
    assert!(image_element.char_payload().is_none());
    assert!(image_element.span_payload().is_none());
    assert!(image_element.image_payload().is_some());

    let descriptor = FontDescriptor {
        id: font_id,
        name: "Helvetica".to_string(),
    };
    assert_eq!(descriptor.id.get(), 1);
}
