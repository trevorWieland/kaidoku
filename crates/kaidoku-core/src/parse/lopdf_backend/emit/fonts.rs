use crate::ExtractError;
use lopdf::{Dictionary, Document, Encoding, Object, ObjectId};
use std::collections::BTreeMap;

const DEFAULT_GLYPH_WIDTH_UNITS: f64 = 500.0;

#[derive(Debug)]
pub(super) struct FontCatalog<'a> {
    fonts: BTreeMap<Vec<u8>, FontRuntime<'a>>,
}

#[derive(Debug)]
struct FontRuntime<'a> {
    display_name: String,
    encoding: Option<Encoding<'a>>,
    width_table: WidthTable,
}

#[derive(Debug)]
enum WidthTable {
    OneByte(OneByteWidthTable),
    Cid(CidWidthTable),
    Fallback { default_width: f64 },
}

#[derive(Debug)]
struct OneByteWidthTable {
    first_char: u32,
    widths: Vec<f64>,
    default_width: f64,
}

#[derive(Debug)]
struct CidWidthTable {
    default_width: f64,
    specs: Vec<CidWidthSpec>,
}

#[derive(Debug)]
enum CidWidthSpec {
    Range { start: u32, end: u32, width: f64 },
    Sequential { start: u32, widths: Vec<f64> },
}

impl<'a> FontCatalog<'a> {
    pub(super) fn from_page(
        document: &'a Document,
        page_id: ObjectId,
    ) -> Result<Self, ExtractError> {
        let page_fonts =
            document
                .get_page_fonts(page_id)
                .map_err(|error| ExtractError::PdfParse {
                    reason: error.to_string(),
                })?;

        let mut fonts = BTreeMap::new();
        for (resource_name, font_dict) in page_fonts {
            let display_name = font_dict
                .get(b"BaseFont")
                .ok()
                .and_then(|value| value.as_name().ok())
                .map_or_else(
                    || String::from_utf8_lossy(&resource_name).to_string(),
                    |bytes| String::from_utf8_lossy(bytes).to_string(),
                );

            let encoding = font_dict.get_font_encoding(document).ok();
            let width_table = width_table_for_font(document, font_dict);

            fonts.insert(
                resource_name,
                FontRuntime {
                    display_name,
                    encoding,
                    width_table,
                },
            );
        }

        Ok(Self { fonts })
    }

    #[must_use]
    pub(super) fn decode_text(&self, font_key: Option<&[u8]>, bytes: &[u8]) -> String {
        if let Some(key) = font_key {
            if let Some(font) = self.fonts.get(key)
                && let Some(encoding) = &font.encoding
                && let Ok(decoded) = Document::decode_text(encoding, bytes)
            {
                return decoded;
            }
        }
        String::from_utf8_lossy(bytes).to_string()
    }

    #[must_use]
    pub(super) fn display_name(&self, font_key: Option<&[u8]>) -> Option<String> {
        font_key
            .and_then(|key| self.fonts.get(key))
            .map(|font| font.display_name.clone())
    }

    #[must_use]
    pub(super) fn glyph_widths(
        &self,
        font_key: Option<&[u8]>,
        bytes: &[u8],
        fallback_len: usize,
    ) -> Vec<f64> {
        if bytes.is_empty() {
            return Vec::new();
        }

        let Some(key) = font_key else {
            return vec![DEFAULT_GLYPH_WIDTH_UNITS; fallback_len.max(1)];
        };
        let Some(font) = self.fonts.get(key) else {
            return vec![DEFAULT_GLYPH_WIDTH_UNITS; fallback_len.max(1)];
        };

        let widths = match &font.width_table {
            WidthTable::OneByte(table) => bytes
                .iter()
                .map(|byte| table.width_for_code(u32::from(*byte)))
                .collect(),
            WidthTable::Cid(table) => decode_cids(bytes)
                .iter()
                .map(|cid| table.width_for_cid(*cid))
                .collect(),
            WidthTable::Fallback { default_width } => vec![*default_width; fallback_len.max(1)],
        };

        if widths.is_empty() {
            vec![DEFAULT_GLYPH_WIDTH_UNITS; fallback_len.max(1)]
        } else {
            widths
        }
    }
}

impl OneByteWidthTable {
    fn width_for_code(&self, code: u32) -> f64 {
        if code < self.first_char {
            return self.default_width;
        }

        let raw_idx = code - self.first_char;
        let Ok(index) = usize::try_from(raw_idx) else {
            return self.default_width;
        };
        self.widths
            .get(index)
            .copied()
            .unwrap_or(self.default_width)
    }
}

impl CidWidthTable {
    fn width_for_cid(&self, cid: u32) -> f64 {
        for spec in &self.specs {
            match spec {
                CidWidthSpec::Range { start, end, width } if cid >= *start && cid <= *end => {
                    return *width;
                }
                CidWidthSpec::Sequential { start, widths } => {
                    let Ok(offset) = usize::try_from(cid.saturating_sub(*start)) else {
                        continue;
                    };
                    if let Some(width) = widths.get(offset) {
                        return *width;
                    }
                }
                CidWidthSpec::Range { .. } => {}
            }
        }

        self.default_width
    }
}

fn width_table_for_font(document: &Document, font_dict: &Dictionary) -> WidthTable {
    if let Some(table) = simple_width_table(font_dict) {
        return WidthTable::OneByte(table);
    }

    if let Some(table) = cid_width_table(document, font_dict) {
        return WidthTable::Cid(table);
    }

    WidthTable::Fallback {
        default_width: missing_width(document, font_dict).unwrap_or(DEFAULT_GLYPH_WIDTH_UNITS),
    }
}

fn simple_width_table(font_dict: &Dictionary) -> Option<OneByteWidthTable> {
    let widths = font_dict.get(b"Widths").ok()?.as_array().ok()?;
    let first_char = object_to_u32(font_dict.get(b"FirstChar").ok()?)?;
    let parsed_widths = widths
        .iter()
        .filter_map(object_to_f64)
        .collect::<Vec<f64>>();

    Some(OneByteWidthTable {
        first_char,
        widths: parsed_widths,
        default_width: DEFAULT_GLYPH_WIDTH_UNITS,
    })
}

fn cid_width_table(document: &Document, font_dict: &Dictionary) -> Option<CidWidthTable> {
    let descendant = descendant_font(document, font_dict)?;
    let default_width = descendant
        .get(b"DW")
        .ok()
        .and_then(object_to_f64)
        .unwrap_or(1000.0);

    let mut specs = Vec::new();
    if let Ok(widths_object) = descendant.get(b"W")
        && let Ok(widths_array) = widths_object.as_array()
    {
        let mut idx = 0usize;
        while idx < widths_array.len() {
            let start = widths_array.get(idx).and_then(object_to_u32)?;
            idx += 1;

            let Some(next) = widths_array.get(idx) else {
                break;
            };

            if let Ok(array) = next.as_array() {
                let parsed_widths = array.iter().filter_map(object_to_f64).collect::<Vec<f64>>();
                specs.push(CidWidthSpec::Sequential {
                    start,
                    widths: parsed_widths,
                });
                idx += 1;
                continue;
            }

            let end = object_to_u32(next)?;
            let width = widths_array.get(idx + 1).and_then(object_to_f64)?;
            specs.push(CidWidthSpec::Range { start, end, width });
            idx += 2;
        }
    }

    Some(CidWidthTable {
        default_width,
        specs,
    })
}

fn descendant_font<'a>(
    document: &'a Document,
    font_dict: &'a Dictionary,
) -> Option<&'a Dictionary> {
    let descendants = font_dict.get(b"DescendantFonts").ok()?.as_array().ok()?;
    let first = descendants.first()?;
    let object = if let Ok(reference) = first.as_reference() {
        document.get_object(reference).ok()?
    } else {
        first
    };

    object.as_dict().ok()
}

fn missing_width(document: &Document, font_dict: &Dictionary) -> Option<f64> {
    if let Ok(descriptor) = font_dict.get_deref(b"FontDescriptor", document)
        && let Ok(dict) = descriptor.as_dict()
    {
        return dict.get(b"MissingWidth").ok().and_then(object_to_f64);
    }

    descendant_font(document, font_dict)
        .and_then(|descendant| descendant.get_deref(b"FontDescriptor", document).ok())
        .and_then(|descriptor| descriptor.as_dict().ok())
        .and_then(|dict| dict.get(b"MissingWidth").ok())
        .and_then(object_to_f64)
}

fn decode_cids(bytes: &[u8]) -> Vec<u32> {
    let mut cids = Vec::new();
    let mut chunks = bytes.chunks_exact(2);
    for chunk in &mut chunks {
        cids.push((u32::from(chunk[0]) << 8) | u32::from(chunk[1]));
    }

    let remainder = chunks.remainder();
    if let Some(last) = remainder.first() {
        cids.push(u32::from(*last));
    }

    cids
}

fn object_to_u32(value: &Object) -> Option<u32> {
    value
        .as_i64()
        .ok()
        .and_then(|number| u32::try_from(number).ok())
}

fn object_to_f64(value: &Object) -> Option<f64> {
    value.as_float().ok().map(f64::from)
}

#[cfg(test)]
mod tests;
