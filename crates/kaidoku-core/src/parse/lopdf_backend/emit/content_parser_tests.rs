use super::{ExtractionControl, parse_content_operations_bounded};
use crate::{ExtractError, PageNumber};
use lopdf::{Object, StringFormat};

const DEFAULT_TEST_DEPTH: usize = 128;

fn test_page_number() -> PageNumber {
    PageNumber::new(1).expect("valid page number")
}

fn parse(input: &[u8]) -> Result<Vec<lopdf::content::Operation>, ExtractError> {
    parse_with_depth(input, DEFAULT_TEST_DEPTH)
}

fn parse_with_depth(
    input: &[u8],
    max_depth: usize,
) -> Result<Vec<lopdf::content::Operation>, ExtractError> {
    let control = ExtractionControl::new(30_000, None);
    parse_content_operations_bounded(input, test_page_number(), max_depth, &control)
}

#[test]
fn parses_basic_operations_and_literals() {
    let operations = parse(b"q 1 0 0 1 72 720 cm /F1 12 Tf (Hello\\040World) Tj Q").expect("parse");

    assert_eq!(operations.len(), 5);
    assert_eq!(operations[0].operator, "q");
    assert_eq!(operations[1].operator, "cm");
    assert_eq!(operations[2].operator, "Tf");
    assert_eq!(operations[3].operator, "Tj");
    assert_eq!(operations[4].operator, "Q");

    let literal_operand = operations[3]
        .operands
        .first()
        .expect("Tj operation should have one operand");
    assert!(matches!(
        literal_operand,
        Object::String(_, StringFormat::Literal)
    ));
    if let Object::String(value, StringFormat::Literal) = literal_operand {
        assert_eq!(value.as_slice(), b"Hello World");
    }
}

#[test]
fn parses_name_hex_escapes_and_hex_strings() {
    let operations = parse(b"/A#42 12 Tf [<901FA> (A)] TJ").expect("parse");

    assert_eq!(operations.len(), 2);
    let text_font_operands = operations[0].operands.as_slice();
    assert!(matches!(
        text_font_operands,
        [Object::Name(_), Object::Integer(12)]
    ));
    if let [Object::Name(name), Object::Integer(12)] = text_font_operands {
        assert_eq!(name.as_slice(), b"AB");
    }

    let show_text_array_operands = operations[1].operands.as_slice();
    assert!(matches!(show_text_array_operands, [Object::Array(_)]));
    if let [Object::Array(items)] = show_text_array_operands {
        assert_eq!(items.len(), 2);
        assert!(matches!(
            &items[0],
            Object::String(_, StringFormat::Hexadecimal)
        ));
        if let Object::String(bytes, StringFormat::Hexadecimal) = &items[0] {
            assert_eq!(bytes.as_slice(), [0x90, 0x1f, 0xa0]);
        }
    }
}

#[test]
fn parses_inline_image_operation() {
    let operations = parse(b"BI /W 1 /H 1 /BPC 8 /CS /DeviceGray ID \x80\nEI Q").expect("parse");

    assert_eq!(operations.len(), 2);
    assert_eq!(operations[0].operator, "BI");
    assert_eq!(operations[1].operator, "Q");

    let bi_operands = operations[0].operands.as_slice();
    assert!(matches!(bi_operands, [Object::Stream(_)]));
    if let [Object::Stream(stream)] = bi_operands {
        assert_eq!(stream.content, vec![0x80]);
    }
}

#[test]
fn parses_inline_image_with_length_preserves_payload_containing_ei_pattern() {
    // Payload contains " EI " bytes that would confuse a length-unaware scan.
    let mut body = Vec::new();
    body.extend_from_slice(b"BI /Length 7 ID ");
    body.extend_from_slice(b" EI \x00\x01\x02");
    body.extend_from_slice(b" EI Q");

    let operations = parse(&body).expect("parse with /Length should succeed");
    assert_eq!(operations.len(), 2);
    assert_eq!(operations[0].operator, "BI");
    assert_eq!(operations[1].operator, "Q");
    assert!(
        matches!(operations[0].operands.as_slice(), [Object::Stream(_)]),
        "expected single stream operand"
    );
    let [Object::Stream(stream)] = operations[0].operands.as_slice() else {
        return;
    };
    assert_eq!(stream.content.len(), 7);
    assert_eq!(&stream.content[..4], b" EI ");
}

#[test]
fn parses_inline_image_with_eol_guarded_scan_rejects_embedded_ei_without_eol() {
    // "EI" appears mid-payload without preceding EOL, so it should NOT terminate.
    // A real EOL+EI later terminates correctly.
    let mut body = Vec::new();
    body.extend_from_slice(b"BI /W 1 /H 1 ID ABEIXY\nEI Q");

    let operations = parse(&body).expect("parse should succeed with EOL-guarded terminator");
    assert_eq!(operations.len(), 2);
    assert!(
        matches!(operations[0].operands.as_slice(), [Object::Stream(_)]),
        "expected single stream operand"
    );
    let [Object::Stream(stream)] = operations[0].operands.as_slice() else {
        return;
    };
    assert_eq!(stream.content, b"ABEIXY");
}

#[test]
fn filtered_inline_image_without_length_falls_back_with_ws_anchor() {
    // Real-world PDF producers sometimes emit filtered inline images without
    // an explicit `/Length`. The bounded filtered-fallback scan terminates on
    // the canonical `WS EI WS` sequence.
    let body = b"BI /Filter /FlateDecode /W 1 /H 1 ID \x78\x9c\x00\nEI Q";
    let operations = parse(body).expect("filtered inline image should fall back");
    assert_eq!(operations.len(), 2);
    assert_eq!(operations[0].operator, "BI");
    assert_eq!(operations[1].operator, "Q");
    assert!(
        matches!(operations[0].operands.as_slice(), [Object::Stream(_)]),
        "expected single stream operand, got {:?}",
        operations[0].operands
    );
    let [Object::Stream(stream)] = operations[0].operands.as_slice() else {
        return;
    };
    assert_eq!(stream.content, b"\x78\x9c\x00");
}

#[test]
fn filtered_inline_image_fallback_errors_when_no_terminator_within_cap() {
    // A filtered payload where no `WS EI (ws|delim|EOF)` sequence ever appears
    // must still error — we do not silently consume the rest of the stream.
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(b"BI /Filter /FlateDecode /W 1 /H 1 ID ");
    body.extend(std::iter::repeat_n(b'X', 512));

    let err = parse(&body).expect_err("must refuse to guess when no WS EI pattern exists");
    assert!(
        matches!(err, ExtractError::ContentDecode { .. }),
        "expected ContentDecode, got {err:?}"
    );
    let ExtractError::ContentDecode { reason } = err else {
        return;
    };
    assert!(reason.contains("FlateDecode"), "reason: {reason}");
    assert!(
        reason.contains("no whitespace-delimited"),
        "reason: {reason}"
    );
}

#[test]
fn ws_anchored_scan_accepts_space_preceded_ei_without_eol() {
    // Unfiltered inline image whose payload ends with ` EI ` (space anchor,
    // not EOL). A strict EOL-only scan would reject this; the relaxed WS
    // anchor accepts it, matching real producer output.
    let body = b"BI /W 1 /H 1 ID ABC EI Q";
    let operations = parse(body).expect("ws-anchored terminator should succeed");
    assert_eq!(operations.len(), 2);
    assert!(
        matches!(operations[0].operands.as_slice(), [Object::Stream(_)]),
        "expected single stream operand"
    );
    let [Object::Stream(stream)] = operations[0].operands.as_slice() else {
        return;
    };
    assert_eq!(stream.content, b"ABC");
}

#[test]
fn inline_image_declared_length_exceeding_bytes_errors_out() {
    let body = b"BI /Length 99 ID ABC";
    let err = parse(body).expect_err("declared length must be validated");
    assert!(
        matches!(err, ExtractError::ContentDecode { .. }),
        "expected ContentDecode, got {err:?}"
    );
    let ExtractError::ContentDecode { reason } = err else {
        return;
    };
    assert!(reason.contains("length"), "reason: {reason}");
}

#[test]
fn deep_nested_array_exceeding_depth_errors_out() {
    let mut body: Vec<u8> = vec![b'['; 130];
    body.push(b'1');
    body.extend(std::iter::repeat_n(b']', 130));
    body.extend_from_slice(b" TJ");

    let err = parse_with_depth(&body, 128).expect_err("must exceed depth");
    assert!(
        matches!(err, ExtractError::ContentNestingLimitExceeded { .. }),
        "expected ContentNestingLimitExceeded for deep array, got {err:?}"
    );
    let ExtractError::ContentNestingLimitExceeded {
        depth,
        limit,
        page_number,
    } = err
    else {
        return;
    };
    assert!(depth > limit);
    assert_eq!(limit, 128);
    assert_eq!(page_number, 1);
}

#[test]
fn deep_nested_dictionary_exceeding_depth_errors_out() {
    let mut body = Vec::new();
    body.extend_from_slice(b"<<");
    for _ in 0..130 {
        body.extend_from_slice(b"/a <<");
    }
    body.extend_from_slice(b"/k 1");
    for _ in 0..130 {
        body.extend_from_slice(b">>");
    }
    body.extend_from_slice(b">> def");

    let err = parse_with_depth(&body, 128).expect_err("must exceed depth");
    assert!(matches!(
        err,
        ExtractError::ContentNestingLimitExceeded { .. }
    ));
}

#[test]
fn nesting_just_within_limit_parses_successfully() {
    // Build [[[ ... ]]] with exactly 127 levels of nesting; the innermost
    // integer parses at depth 128 which equals the max_depth.
    let depth: usize = 127;
    let mut body: Vec<u8> = vec![b'['; depth];
    body.push(b'1');
    body.extend(std::iter::repeat_n(b']', depth));
    body.extend_from_slice(b" TJ");

    let operations =
        parse_with_depth(&body, 128).expect("127 levels of nesting must parse at max_depth=128");
    assert_eq!(operations.len(), 1);
}

#[test]
fn nesting_one_below_limit_mixing_arrays_and_dicts_parses() {
    // Alternating array/dict nesting well within the 128 limit.
    let body = b"[<< /a [<< /b [<< /c 1 >>] >>] >>] TJ";
    let ops = parse(body).expect("alternating nesting parses");
    assert_eq!(ops.len(), 1);
}

#[test]
fn streaming_api_returns_each_operation_without_buffering() {
    use super::parse_content_operations_streaming;
    let control = ExtractionControl::new(30_000, None);
    let mut count = 0_u32;
    let mut last_index = None;
    parse_content_operations_streaming(
        b"q 1 2 3 4 5 6 cm Q",
        test_page_number(),
        DEFAULT_TEST_DEPTH,
        &control,
        |_op, idx| {
            count += 1;
            last_index = Some(idx);
            Ok(())
        },
    )
    .expect("streaming parse succeeds");
    assert_eq!(count, 3);
    assert_eq!(last_index, Some(2));
}
