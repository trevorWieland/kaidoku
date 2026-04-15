use super::{
    CidWidthSpec, FontCatalog, FontRuntime, OneByteWidthTable, WidthTable, cid_width_table,
    decode_cids, missing_width, simple_width_table, width_table_for_font,
};
use lopdf::{Document, Object, ObjectId, dictionary};
use std::collections::BTreeMap;

#[test]
fn simple_width_table_and_catalog_widths_work() {
    let font_dict = dictionary! {
        "FirstChar" => 32,
        "Widths" => vec![200.into(), 300.into(), 400.into()],
    };
    let table = simple_width_table(&font_dict);
    assert!(table.is_some());
    let Some(table) = table else { return };

    assert!((table.width_for_code(32) - 200.0).abs() < f64::EPSILON);
    assert!((table.width_for_code(34) - 400.0).abs() < f64::EPSILON);
    assert!((table.width_for_code(10) - 500.0).abs() < f64::EPSILON);
}

#[test]
fn cid_width_table_parses_ranges_and_sequences() {
    let mut document = Document::new();
    let descendant_id: ObjectId = (10, 0);
    document.objects.insert(
        descendant_id,
        Object::Dictionary(dictionary! {
            "DW" => 900,
            "W" => vec![
                16.into(),
                Object::Array(vec![500.into(), 510.into()]),
                20.into(),
                22.into(),
                700.into(),
            ],
        }),
    );

    let font_dict = dictionary! {
        "DescendantFonts" => vec![Object::Reference(descendant_id)],
    };
    let table = cid_width_table(&document, &font_dict);
    assert!(table.is_some());
    let Some(table) = table else { return };

    assert!((table.width_for_cid(16) - 500.0).abs() < f64::EPSILON);
    assert!((table.width_for_cid(17) - 510.0).abs() < f64::EPSILON);
    assert!((table.width_for_cid(21) - 700.0).abs() < f64::EPSILON);
    assert!((table.width_for_cid(40) - 900.0).abs() < f64::EPSILON);
}

#[test]
fn missing_width_prefers_font_descriptor_entries() {
    let mut document = Document::new();
    let descriptor_id: ObjectId = (11, 0);
    document.objects.insert(
        descriptor_id,
        Object::Dictionary(dictionary! {
            "MissingWidth" => 333,
        }),
    );

    let font_dict = dictionary! {
        "FontDescriptor" => Object::Reference(descriptor_id),
    };
    let missing = missing_width(&document, &font_dict);
    assert!(missing.is_some());
    let Some(missing) = missing else { return };
    assert!((missing - 333.0).abs() < f64::EPSILON);
}

#[test]
fn decode_cids_supports_even_and_odd_byte_inputs() {
    let even = decode_cids(&[0x00, 0x10, 0x00, 0x20]);
    assert_eq!(even, vec![16, 32]);

    let odd = decode_cids(&[0x00, 0x10, 0x7f]);
    assert_eq!(odd, vec![16, 127]);
}

#[test]
fn catalog_decodes_and_returns_widths_for_known_and_unknown_fonts() {
    let mut fonts = BTreeMap::new();
    fonts.insert(
        b"F1".to_vec(),
        FontRuntime {
            display_name: "UnitTestFont".to_string(),
            encoding: None,
            width_table: WidthTable::OneByte(OneByteWidthTable {
                first_char: 65,
                widths: vec![450.0, 550.0],
                default_width: 500.0,
            }),
        },
    );
    fonts.insert(
        b"CID".to_vec(),
        FontRuntime {
            display_name: "UnitTestCid".to_string(),
            encoding: None,
            width_table: WidthTable::Cid(super::CidWidthTable {
                default_width: 600.0,
                specs: vec![
                    CidWidthSpec::Range {
                        start: 10,
                        end: 10,
                        width: 300.0,
                    },
                    CidWidthSpec::Sequential {
                        start: 16,
                        widths: vec![400.0],
                    },
                ],
            }),
        },
    );

    let catalog = FontCatalog { fonts };
    let decoded = catalog.decode_text(Some(b"F1"), b"AB");
    assert_eq!(decoded, "AB");

    let widths = catalog.glyph_widths(Some(b"F1"), b"AB", 2);
    assert_eq!(widths, vec![450.0, 550.0]);

    let cid_widths = catalog.glyph_widths(Some(b"CID"), &[0x00, 0x10, 0x00, 0x0a], 2);
    assert_eq!(cid_widths, vec![400.0, 300.0]);

    let fallback_widths = catalog.glyph_widths(Some(b"MISSING"), b"ZZ", 2);
    assert_eq!(fallback_widths, vec![500.0, 500.0]);
    assert_eq!(
        catalog.display_name(Some(b"F1")),
        Some("UnitTestFont".to_string())
    );
    assert_eq!(catalog.display_name(Some(b"MISSING")), None);
}

#[test]
fn from_page_builds_catalog_from_page_resources() {
    let mut document = Document::new();
    let page_id: ObjectId = (1, 0);
    let resources_id: ObjectId = (2, 0);
    let font_id: ObjectId = (3, 0);

    document.objects.insert(
        font_id,
        Object::Dictionary(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
            "FirstChar" => 65,
            "Widths" => vec![500.into()],
        }),
    );
    document.objects.insert(
        resources_id,
        Object::Dictionary(dictionary! {
            "Font" => dictionary! {
                "F1" => Object::Reference(font_id),
            },
        }),
    );
    document.objects.insert(
        page_id,
        Object::Dictionary(dictionary! {
            "Type" => "Page",
            "Resources" => Object::Reference(resources_id),
        }),
    );

    let catalog = FontCatalog::from_page(&document, page_id);
    assert!(catalog.is_ok());
    let Ok(catalog) = catalog else { return };

    let widths = catalog.glyph_widths(Some(b"F1"), b"A", 1);
    assert_eq!(widths, vec![500.0]);
    assert_eq!(
        catalog.display_name(Some(b"F1")),
        Some("Helvetica".to_string())
    );
}

#[test]
fn width_table_for_font_falls_back_to_missing_width() {
    let mut document = Document::new();
    let descriptor_id: ObjectId = (20, 0);
    document.objects.insert(
        descriptor_id,
        Object::Dictionary(dictionary! {
            "MissingWidth" => 321,
        }),
    );

    let font_dict = dictionary! {
        "FontDescriptor" => Object::Reference(descriptor_id),
    };
    let width_table = width_table_for_font(&document, &font_dict);
    let WidthTable::Fallback { default_width } = width_table else {
        return;
    };
    assert!((default_width - 321.0).abs() < f64::EPSILON);
}
