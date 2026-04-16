use super::{
    DecodedBudget, ExtractionControl, ExtractionLimits, FontRegistry, FontRegistryAccess,
    PageEmitConfig, extract_page_elements,
};
use crate::PageNumber;
use crate::parse::lopdf_backend::resources::{page_geometry, page_resource_scope};
use lopdf::{Document, Object, ObjectId, Stream, dictionary};
use std::collections::BTreeSet;

#[test]
fn tj_elements_have_unique_source_ref_indices() {
    let (document, page_id) = page_document_with_streams(&[b"BT 10 20 Td [(AB) 120 (CD)] TJ ET"]);
    let page_number = PageNumber::new(1);
    assert!(page_number.is_ok());
    let Ok(page_number) = page_number else { return };

    let geometry = page_geometry(&document, page_number, page_id, 32);
    assert!(geometry.is_ok());
    let Ok(geometry) = geometry else { return };
    let scope = page_resource_scope(&document, page_id);
    assert!(scope.is_ok());
    let Ok(scope) = scope else { return };
    let scope = std::sync::Arc::new(scope);
    let control = ExtractionControl::new(30_000, None);
    let mut font_registry = FontRegistry::default();
    let decoded_budget = DecodedBudget::new(10_000_000);

    let elements = extract_page_elements(
        &document,
        page_number,
        page_id,
        PageEmitConfig {
            coordinate_precision: 3,
            page_geometry: geometry,
            root_scope: &scope,
            limits: ExtractionLimits {
                operation_budget: 10_000,
                max_elements: 10_000,
                stream_byte_limit: 1_000_000,
                max_form_depth: 8,
                max_form_visits: 128,
                max_content_nesting_depth: 128,
            },
            control: &control,
        },
        FontRegistryAccess::Mutable(&mut font_registry),
        &decoded_budget,
    );
    assert!(
        elements.is_ok(),
        "expected extraction success, got error: {elements:?}"
    );
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

    let geometry = page_geometry(&document, page_number, page_id, 32);
    assert!(geometry.is_ok());
    let Ok(geometry) = geometry else { return };
    let scope = page_resource_scope(&document, page_id);
    assert!(scope.is_ok());
    let Ok(scope) = scope else { return };
    let scope = std::sync::Arc::new(scope);
    let control = ExtractionControl::new(30_000, None);
    let mut font_registry = FontRegistry::default();
    let decoded_budget = DecodedBudget::new(10_000_000);

    let elements = extract_page_elements(
        &document,
        page_number,
        page_id,
        PageEmitConfig {
            coordinate_precision: 3,
            page_geometry: geometry,
            root_scope: &scope,
            limits: ExtractionLimits {
                operation_budget: 10_000,
                max_elements: 10_000,
                stream_byte_limit: 1_000_000,
                max_form_depth: 8,
                max_form_visits: 128,
                max_content_nesting_depth: 128,
            },
            control: &control,
        },
        FontRegistryAccess::Mutable(&mut font_registry),
        &decoded_budget,
    );
    assert!(
        elements.is_ok(),
        "expected extraction success, got error: {elements:?}"
    );
    let Ok(elements) = elements else { return };

    let span = elements.iter().find(|element| {
        element
            .span_payload()
            .is_some_and(|payload| payload.text() == "A")
    });
    assert!(span.is_some());
    let Some(span) = span else { return };

    assert!(
        span.bbox().x() >= 9.0,
        "expected carried text position from prior stream, got x={}",
        span.bbox().x()
    );
}

#[test]
fn inline_image_bi_emits_image_element() {
    // Use declared /Length so the inline-image boundary is unambiguous: the
    // 18-byte payload is consumed exactly, then `EI` follows unconditionally.
    let inline_content =
        b"q 10 0 0 20 5 7 cm BI /W 2 /H 3 /CS /RGB /BPC 8 /Length 18 ID 123456789012345678 EI Q";
    let (document, page_id) = page_document_with_streams(&[inline_content]);
    let page_number = PageNumber::new(1).expect("page number");

    let geometry = page_geometry(&document, page_number, page_id, 32).expect("geometry");
    let scope = std::sync::Arc::new(page_resource_scope(&document, page_id).expect("scope"));
    let control = ExtractionControl::new(30_000, None);
    let mut font_registry = FontRegistry::default();
    let decoded_budget = DecodedBudget::new(10_000_000);

    let elements = extract_page_elements(
        &document,
        page_number,
        page_id,
        PageEmitConfig {
            coordinate_precision: 3,
            page_geometry: geometry,
            root_scope: &scope,
            limits: ExtractionLimits {
                operation_budget: 10_000,
                max_elements: 10_000,
                stream_byte_limit: 1_000_000,
                max_form_depth: 8,
                max_form_visits: 128,
                max_content_nesting_depth: 128,
            },
            control: &control,
        },
        FontRegistryAccess::Mutable(&mut font_registry),
        &decoded_budget,
    )
    .expect("extract");

    let image = elements
        .iter()
        .find_map(|element| element.image_payload())
        .expect("inline image payload");

    assert_eq!(image.width_px, 2);
    assert_eq!(image.height_px, 3);
    assert!(image.name.starts_with("inline_"));
}

#[test]
fn form_xobject_recursion_emits_nested_image() {
    let mut document = Document::new();

    let page_id: ObjectId = (1, 0);
    let content_id: ObjectId = (2, 0);
    let resources_id: ObjectId = (3, 0);
    let form_id: ObjectId = (4, 0);
    let form_resources_id: ObjectId = (5, 0);
    let image_id: ObjectId = (6, 0);

    document.objects.insert(
        image_id,
        Object::Stream(Stream::new(
            dictionary! {
                "Subtype" => "Image",
                "Width" => 7,
                "Height" => 9,
                "ColorSpace" => "DeviceRGB",
                "BitsPerComponent" => 8,
            },
            vec![],
        )),
    );

    document.objects.insert(
        form_resources_id,
        Object::Dictionary(dictionary! {
            "XObject" => dictionary! {
                "NestedIm" => Object::Reference(image_id),
            },
        }),
    );

    let form_stream = Stream::new(
        dictionary! {
            "Subtype" => "Form",
            "Resources" => Object::Reference(form_resources_id),
            "Matrix" => vec![1.into(), 0.into(), 0.into(), 1.into(), 3.into(), 4.into()],
        },
        b"q 1 0 0 1 10 20 cm /NestedIm Do Q".to_vec(),
    );
    document
        .objects
        .insert(form_id, Object::Stream(form_stream));

    document.objects.insert(
        resources_id,
        Object::Dictionary(dictionary! {
            "XObject" => dictionary! {
                "Fm1" => Object::Reference(form_id),
            },
        }),
    );

    document.objects.insert(
        content_id,
        Object::Stream(Stream::new(dictionary! {}, b"/Fm1 Do".to_vec())),
    );

    document.objects.insert(
        page_id,
        Object::Dictionary(dictionary! {
            "Type" => "Page",
            "Resources" => Object::Reference(resources_id),
            "Contents" => Object::Reference(content_id),
            "MediaBox" => vec![0.into(), 0.into(), 200.into(), 200.into()],
        }),
    );

    let page_number = PageNumber::new(1).expect("page number");
    let geometry = page_geometry(&document, page_number, page_id, 32).expect("geometry");
    let scope = std::sync::Arc::new(page_resource_scope(&document, page_id).expect("scope"));
    let control = ExtractionControl::new(30_000, None);
    let mut font_registry = FontRegistry::default();
    let decoded_budget = DecodedBudget::new(10_000_000);

    let elements = extract_page_elements(
        &document,
        page_number,
        page_id,
        PageEmitConfig {
            coordinate_precision: 3,
            page_geometry: geometry,
            root_scope: &scope,
            limits: ExtractionLimits {
                operation_budget: 10_000,
                max_elements: 10_000,
                stream_byte_limit: 1_000_000,
                max_form_depth: 8,
                max_form_visits: 128,
                max_content_nesting_depth: 128,
            },
            control: &control,
        },
        FontRegistryAccess::Mutable(&mut font_registry),
        &decoded_budget,
    )
    .expect("extract");

    let image = elements
        .iter()
        .find_map(|element| element.image_payload())
        .expect("form image payload");
    assert_eq!(image.name, "NestedIm");
}

#[test]
fn form_cycle_is_reported_as_structural_error() {
    let mut document = Document::new();

    let page_id: ObjectId = (1, 0);
    let content_id: ObjectId = (2, 0);
    let resources_id: ObjectId = (3, 0);
    let form_id: ObjectId = (4, 0);
    let form_resources_id: ObjectId = (5, 0);

    document.objects.insert(
        form_resources_id,
        Object::Dictionary(dictionary! {
            "XObject" => dictionary! {
                "Self" => Object::Reference(form_id),
            },
        }),
    );

    let form_stream = Stream::new(
        dictionary! {
            "Subtype" => "Form",
            "Resources" => Object::Reference(form_resources_id),
        },
        b"/Self Do".to_vec(),
    );
    document
        .objects
        .insert(form_id, Object::Stream(form_stream));

    document.objects.insert(
        resources_id,
        Object::Dictionary(dictionary! {
            "XObject" => dictionary! {
                "Fm1" => Object::Reference(form_id),
            },
        }),
    );

    document.objects.insert(
        content_id,
        Object::Stream(Stream::new(dictionary! {}, b"/Fm1 Do".to_vec())),
    );

    document.objects.insert(
        page_id,
        Object::Dictionary(dictionary! {
            "Type" => "Page",
            "Resources" => Object::Reference(resources_id),
            "Contents" => Object::Reference(content_id),
            "MediaBox" => vec![0.into(), 0.into(), 200.into(), 200.into()],
        }),
    );

    let page_number = PageNumber::new(1).expect("page number");
    let geometry = page_geometry(&document, page_number, page_id, 32).expect("geometry");
    let scope = std::sync::Arc::new(page_resource_scope(&document, page_id).expect("scope"));
    let control = ExtractionControl::new(30_000, None);
    let mut font_registry = FontRegistry::default();
    let decoded_budget = DecodedBudget::new(10_000_000);

    let error = extract_page_elements(
        &document,
        page_number,
        page_id,
        PageEmitConfig {
            coordinate_precision: 3,
            page_geometry: geometry,
            root_scope: &scope,
            limits: ExtractionLimits {
                operation_budget: 10_000,
                max_elements: 10_000,
                stream_byte_limit: 1_000_000,
                max_form_depth: 8,
                max_form_visits: 128,
                max_content_nesting_depth: 128,
            },
            control: &control,
        },
        FontRegistryAccess::Mutable(&mut font_registry),
        &decoded_budget,
    )
    .expect_err("cycle should fail");

    assert!(matches!(
        error,
        crate::ExtractError::FormXObjectCycleDetected { .. }
    ));
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
            "MediaBox" => vec![0.into(), 0.into(), 500.into(), 700.into()],
        }),
    );

    (document, page_id)
}
