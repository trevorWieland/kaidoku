use super::{ExtractionControl, parse_content_operations_bounded};
use crate::PageNumber;
use lopdf::{Object, StringFormat};

fn test_page_number() -> PageNumber {
    PageNumber::new(1).expect("valid page number")
}

#[test]
fn parses_basic_operations_and_literals() {
    let control = ExtractionControl::new(30_000, None);
    let operations = parse_content_operations_bounded(
        b"q 1 0 0 1 72 720 cm /F1 12 Tf (Hello\\040World) Tj Q",
        test_page_number(),
        &control,
    )
    .expect("content parse should succeed");

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
    let control = ExtractionControl::new(30_000, None);
    let operations = parse_content_operations_bounded(
        b"/A#42 12 Tf [<901FA> (A)] TJ",
        test_page_number(),
        &control,
    )
    .expect("content parse should succeed");

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
    let control = ExtractionControl::new(30_000, None);
    let operations = parse_content_operations_bounded(
        b"BI /W 1 /H 1 /BPC 8 /CS /DeviceGray ID \x80 EI Q",
        test_page_number(),
        &control,
    )
    .expect("content parse should succeed");

    assert_eq!(operations.len(), 2);
    assert_eq!(operations[0].operator, "BI");
    assert_eq!(operations[1].operator, "Q");

    let bi_operands = operations[0].operands.as_slice();
    assert!(matches!(bi_operands, [Object::Stream(_)]));
    if let [Object::Stream(stream)] = bi_operands {
        assert_eq!(stream.content, vec![0x80]);
        assert_eq!(
            stream
                .dict
                .get(b"W")
                .and_then(Object::as_i64)
                .expect("inline image width"),
            1
        );
        assert_eq!(
            stream
                .dict
                .get(b"H")
                .and_then(Object::as_i64)
                .expect("inline image height"),
            1
        );
    }
}
