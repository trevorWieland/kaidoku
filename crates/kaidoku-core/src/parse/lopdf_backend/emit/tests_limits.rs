use super::{
    ExtractionControl, ExtractionLimits, FontRegistry, PageEmitConfig, extract_page_elements,
};
use crate::PageNumber;
use crate::parse::lopdf_backend::resources::{page_geometry, page_resource_scope};
use flate2::Compression;
use flate2::write::ZlibEncoder;
use lopdf::{Document, Object, ObjectId, Stream, dictionary};
use std::io::Write;

#[test]
fn compressed_stream_limit_is_enforced_during_decode() {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    let large_plain = b"BT "
        .iter()
        .cycle()
        .take(400_000)
        .copied()
        .collect::<Vec<u8>>();
    encoder
        .write_all(&large_plain)
        .expect("compressed stream input");
    let compressed = encoder.finish().expect("compressed bytes");

    let mut document = Document::new();
    let page_id: ObjectId = (1, 0);
    let stream_id: ObjectId = (2, 0);

    document.objects.insert(
        stream_id,
        Object::Stream(Stream::new(
            dictionary! {
                "Filter" => "FlateDecode",
            },
            compressed,
        )),
    );

    document.objects.insert(
        page_id,
        Object::Dictionary(dictionary! {
            "Type" => "Page",
            "Contents" => Object::Reference(stream_id),
            "MediaBox" => vec![0.into(), 0.into(), 200.into(), 200.into()],
        }),
    );

    let page_number = PageNumber::new(1).expect("page number");
    let geometry = page_geometry(&document, page_number, page_id, 32).expect("geometry");
    let scope = page_resource_scope(&document, page_id).expect("scope");
    let control = ExtractionControl::new(30_000, None);
    let mut font_registry = FontRegistry::default();
    let mut remaining_budget = 10_000_000;

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
                stream_byte_limit: 1024,
                total_stream_budget: 10_000_000,
                max_form_depth: 8,
                max_form_visits: 128,
            },
            control: &control,
        },
        &mut font_registry,
        &mut remaining_budget,
    )
    .expect_err("decoded stream limit should fail");

    assert!(matches!(
        error,
        crate::ExtractError::ContentStreamDecodeLimitExceeded { .. }
    ));
}

#[test]
fn cumulative_decoded_stream_budget_is_enforced() {
    let mut document = Document::new();
    let page_id: ObjectId = (1, 0);
    let first_stream_id: ObjectId = (2, 0);
    let second_stream_id: ObjectId = (3, 0);

    document.objects.insert(
        first_stream_id,
        Object::Stream(Stream::new(
            dictionary! {},
            b"BT 10 20 Td (A) Tj ET".repeat(10),
        )),
    );
    document.objects.insert(
        second_stream_id,
        Object::Stream(Stream::new(
            dictionary! {},
            b"BT 10 20 Td (B) Tj ET".repeat(10),
        )),
    );

    document.objects.insert(
        page_id,
        Object::Dictionary(dictionary! {
            "Type" => "Page",
            "Contents" => Object::Array(vec![Object::Reference(first_stream_id), Object::Reference(second_stream_id)]),
            "MediaBox" => vec![0.into(), 0.into(), 200.into(), 200.into()],
        }),
    );

    let page_number = PageNumber::new(1).expect("page number");
    let geometry = page_geometry(&document, page_number, page_id, 32).expect("geometry");
    let scope = page_resource_scope(&document, page_id).expect("scope");
    let control = ExtractionControl::new(30_000, None);
    let mut font_registry = FontRegistry::default();
    let mut remaining_budget = 200;

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
                total_stream_budget: 200,
                max_form_depth: 8,
                max_form_visits: 128,
            },
            control: &control,
        },
        &mut font_registry,
        &mut remaining_budget,
    )
    .expect_err("cumulative budget should fail");

    assert!(matches!(
        error,
        crate::ExtractError::DecodedStreamBudgetExceeded { .. }
    ));
}
