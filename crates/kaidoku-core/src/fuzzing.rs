use crate::parse::lopdf_backend::emit::{
    ExtractionControl, content_parser_bounded_for_fuzz, decode::decode_content_stream_bounded,
};
use crate::parse::lopdf_backend::resources::page_geometry;
use crate::{BBox, PageNumber};
use lopdf::{Document, Object, ObjectId, Stream, content::Content, dictionary};

pub fn fuzz_decode_filter_path(data: &[u8]) {
    let Some(page_number) = PageNumber::new(1).ok() else {
        return;
    };

    let filter = match data.first().copied().map(|value| value % 5) {
        Some(0) => "FlateDecode",
        Some(1) => "LZWDecode",
        Some(2) => "ASCII85Decode",
        Some(3) => "ASCIIHexDecode",
        _ => "RunLengthDecode",
    };

    let payload = data.get(1..).unwrap_or_default().to_vec();
    let stream = Stream::new(
        dictionary! {
            "Filter" => filter,
        },
        payload,
    );

    let control = ExtractionControl::new(5_000, None);
    let _ = decode_content_stream_bounded(&stream, page_number, 0, 1024 * 1024, &control);
}

pub fn fuzz_content_ops_path(data: &[u8]) {
    let _ = Content::decode(data);
}

/// Drive the custom kaidoku content-stream parser with arbitrary input.
///
/// Unlike `fuzz_content_ops_path` (which exercises lopdf's decoder), this
/// harness targets the hardened parser that runs in production: depth-limited
/// object nesting, spec-hardened inline-image EI termination, and streaming
/// dispatch. It is the only fuzz target that can surface regressions in those
/// paths.
pub fn fuzz_content_parser_path(data: &[u8]) {
    let Some(page_number) = PageNumber::new(1).ok() else {
        return;
    };
    let control = ExtractionControl::new(5_000, None);
    let _ = content_parser_bounded_for_fuzz(data, page_number, 128, &control);
}

pub fn fuzz_geometry_path(data: &[u8]) {
    let Some(page_number) = PageNumber::new(1).ok() else {
        return;
    };

    let mut document = Document::new();
    let page_id: ObjectId = (1, 0);

    let rotate = data
        .get(0..4)
        .map(|bytes| i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        .unwrap_or(0);
    let width = data
        .get(4)
        .copied()
        .map_or(200_i32, |value| i32::from(value) + 1);
    let height = data
        .get(5)
        .copied()
        .map_or(300_i32, |value| i32::from(value) + 1);

    document.objects.insert(
        page_id,
        Object::Dictionary(dictionary! {
            "Type" => "Page",
            "MediaBox" => vec![0.into(), 0.into(), width.into(), height.into()],
            "Rotate" => rotate,
        }),
    );

    let geometry = page_geometry(&document, page_number, page_id, 32);
    if let Ok(geometry) = geometry
        && let Ok(bbox) = BBox::new(0.0, 0.0, 10.0, 12.0)
    {
        let _ = geometry.normalize_bbox(bbox, 3, page_number);
    }
}
