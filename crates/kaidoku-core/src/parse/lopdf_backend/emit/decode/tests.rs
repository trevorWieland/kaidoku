use super::{
    ExtractionControl, decode_ascii_hex_bounded, decode_lzw_bounded, decode_run_length_bounded,
    decode_zlib_bounded,
};
use crate::PageNumber;
use weezl::{BitOrder, encode::Encoder as LzwEncoder};

#[test]
fn ascii_hex_decode_supports_odd_nibble_tail() {
    let page_number = PageNumber::new(1).expect("page number");
    let control = ExtractionControl::new(10_000, None);
    let decoded =
        decode_ascii_hex_bounded(b"61 62 63F>", page_number, 0, 1024, &control).expect("decode");
    assert_eq!(decoded, b"abc\xf0");
}

#[test]
fn run_length_decode_supports_literal_and_repeat_runs() {
    let page_number = PageNumber::new(1).expect("page number");
    let control = ExtractionControl::new(10_000, None);
    let input = [2, b'A', b'B', b'C', 255, b'Z', 128];
    let decoded =
        decode_run_length_bounded(&input, page_number, 0, 1024, &control).expect("decode");
    assert_eq!(decoded, b"ABCZZ");
}

#[test]
fn decode_checks_stream_limit() {
    let page_number = PageNumber::new(1).expect("page number");
    let control = ExtractionControl::new(10_000, None);
    let result = decode_ascii_hex_bounded(b"61626364>", page_number, 0, 2, &control);
    assert!(matches!(
        result,
        Err(crate::ExtractError::ContentStreamDecodeLimitExceeded { .. })
    ));
}

#[test]
fn decode_respects_timeout_guard() {
    let page_number = PageNumber::new(1).expect("page number");
    let control = ExtractionControl::new(0, None);
    let result = decode_zlib_bounded(
        &[120, 156, 3, 0, 0, 0, 0, 1],
        page_number,
        0,
        1024,
        &control,
    );
    assert!(matches!(
        result,
        Err(crate::ExtractError::ExtractionTimeoutExceeded { .. })
    ));
}

#[test]
fn lzw_decode_honors_cancellation_during_decode() {
    let page_number = PageNumber::new(1).expect("page number");
    let plain = b"HELLO LZW";
    let mut encoder = LzwEncoder::new(BitOrder::Msb, 8);
    let encoded_bytes = encoder.encode(plain).expect("encode");

    let control = ExtractionControl::new(0, None);
    let result = decode_lzw_bounded(&encoded_bytes, None, page_number, 0, usize::MAX, &control);
    assert!(matches!(
        result,
        Err(crate::ExtractError::ExtractionTimeoutExceeded {
            stage: "decode_lzw_chunk",
            ..
        })
    ));
}
