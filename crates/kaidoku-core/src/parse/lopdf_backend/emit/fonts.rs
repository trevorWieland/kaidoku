use crate::ExtractError;
use lopdf::{Dictionary, Document, Encoding, Object, ObjectId};
use std::collections::BTreeMap;

const DEFAULT_GLYPH_WIDTH_UNITS: f64 = 500.0;

#[derive(Debug)]
pub(super) struct FontCatalog<'a> {
    fonts: BTreeMap<Vec<u8>, FontRuntime<'a>>,
}

#[derive(Debug, Clone)]
pub(super) struct GlyphRun {
    pub(super) text: String,
    pub(super) width_units: f64,
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
    pub(super) fn display_name(&self, font_key: Option<&[u8]>) -> Option<&str> {
        font_key
            .and_then(|key| self.fonts.get(key))
            .map(|font| font.display_name.as_str())
    }

    /// Deterministic iterator of all display names on this page, ordered by
    /// the underlying font resource name (`BTreeMap` key ordering).
    pub(super) fn iter_display_names(&self) -> impl Iterator<Item = &str> {
        self.fonts.values().map(|font| font.display_name.as_str())
    }

    #[must_use]
    pub(super) fn glyph_runs(&self, font_key: Option<&[u8]>, bytes: &[u8]) -> Vec<GlyphRun> {
        if bytes.is_empty() {
            return Vec::new();
        }

        let Some(key) = font_key else {
            return fallback_runs_from_bytes(bytes, DEFAULT_GLYPH_WIDTH_UNITS);
        };
        let Some(font) = self.fonts.get(key) else {
            return fallback_runs_from_bytes(bytes, DEFAULT_GLYPH_WIDTH_UNITS);
        };

        let code_units = decode_code_units_for_font(font, bytes);
        if code_units.is_empty() {
            return fallback_runs_from_bytes(bytes, DEFAULT_GLYPH_WIDTH_UNITS);
        }

        let mut runs = Vec::with_capacity(code_units.len());
        for unit in code_units {
            let mut text = decode_unit_text(font.encoding.as_ref(), &unit);
            if text.is_empty() {
                text = String::from_utf8_lossy(&unit).to_string();
            }

            let width_units = match &font.width_table {
                WidthTable::OneByte(table) => table.width_for_code(u32::from(unit[0])),
                WidthTable::Cid(table) => table.width_for_cid(code_unit_to_u32(&unit)),
                WidthTable::Fallback { default_width } => *default_width,
            };

            runs.push(GlyphRun { text, width_units });
        }

        if runs.is_empty() {
            fallback_runs_from_bytes(bytes, DEFAULT_GLYPH_WIDTH_UNITS)
        } else {
            runs
        }
    }
}

fn decode_unit_text(encoding: Option<&Encoding<'_>>, unit: &[u8]) -> String {
    if let Some(encoding) = encoding
        && let Ok(text) = Document::decode_text(encoding, unit)
    {
        return text;
    }
    String::new()
}

fn fallback_runs_from_bytes(bytes: &[u8], width_units: f64) -> Vec<GlyphRun> {
    bytes
        .iter()
        .map(|byte| GlyphRun {
            text: String::from_utf8_lossy(&[*byte]).to_string(),
            width_units,
        })
        .collect()
}

fn decode_code_units_for_font(font: &FontRuntime<'_>, bytes: &[u8]) -> Vec<Vec<u8>> {
    match &font.width_table {
        WidthTable::OneByte(_) => bytes.iter().map(|byte| vec![*byte]).collect(),
        WidthTable::Cid(_) => decode_cid_code_units(bytes, font.encoding.as_ref()),
        WidthTable::Fallback { .. } => decode_variable_code_units(bytes, font.encoding.as_ref(), 1),
    }
}

fn decode_cid_code_units(bytes: &[u8], encoding: Option<&Encoding<'_>>) -> Vec<Vec<u8>> {
    let variable_units = decode_variable_code_units(bytes, encoding, 2);
    if variable_units.is_empty() {
        return Vec::new();
    }

    let mut normalized = Vec::new();
    for unit in variable_units {
        if unit.len() <= 2 {
            normalized.push(unit);
            continue;
        }

        let mut chunks = unit.chunks_exact(2);
        for chunk in &mut chunks {
            normalized.push(chunk.to_vec());
        }
        let remainder = chunks.remainder();
        if !remainder.is_empty() {
            normalized.push(remainder.to_vec());
        }
    }
    normalized
}

fn decode_variable_code_units(
    bytes: &[u8],
    encoding: Option<&Encoding<'_>>,
    fallback_unit: usize,
) -> Vec<Vec<u8>> {
    if bytes.is_empty() {
        return Vec::new();
    }

    let Some(encoding) = encoding else {
        return bytes
            .chunks(fallback_unit.max(1))
            .map(<[u8]>::to_vec)
            .collect();
    };

    let mut units = Vec::new();
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        let remaining = bytes.len().saturating_sub(cursor);
        let max_len = remaining.min(4);
        let mut selected: Option<Vec<u8>> = None;

        for width in (1..=max_len).rev() {
            let candidate = &bytes[cursor..cursor + width];
            let decoded = Document::decode_text(encoding, candidate);
            let Ok(text) = decoded else {
                continue;
            };
            if text.is_empty() || text.chars().any(|character| character == '\u{FFFD}') {
                continue;
            }
            selected = Some(candidate.to_vec());
            break;
        }

        if let Some(unit) = selected {
            cursor = cursor.saturating_add(unit.len());
            units.push(unit);
        } else {
            let width = fallback_unit.min(remaining).max(1);
            let unit = bytes[cursor..cursor + width].to_vec();
            cursor = cursor.saturating_add(unit.len());
            units.push(unit);
        }
    }

    units
}

fn code_unit_to_u32(unit: &[u8]) -> u32 {
    unit.iter()
        .fold(0_u32, |acc, byte| (acc << 8) | u32::from(*byte))
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
    // Resolve MissingWidth once and thread it into every path — previously
    // the simple-width table ignored it and used a hardcoded 500.0, causing
    // bbox drift for unsupported code points.
    let resolved_missing_width = missing_width(document, font_dict);

    if let Some(table) = simple_width_table(font_dict, resolved_missing_width) {
        return WidthTable::OneByte(table);
    }

    if let Some(table) = cid_width_table(document, font_dict, resolved_missing_width) {
        return WidthTable::Cid(table);
    }

    WidthTable::Fallback {
        default_width: resolved_missing_width.unwrap_or(DEFAULT_GLYPH_WIDTH_UNITS),
    }
}

fn simple_width_table(
    font_dict: &Dictionary,
    resolved_missing_width: Option<f64>,
) -> Option<OneByteWidthTable> {
    let widths = font_dict.get(b"Widths").ok()?.as_array().ok()?;
    let first_char = object_to_u32(font_dict.get(b"FirstChar").ok()?)?;
    let parsed_widths = widths
        .iter()
        .filter_map(object_to_f64)
        .collect::<Vec<f64>>();

    Some(OneByteWidthTable {
        first_char,
        widths: parsed_widths,
        default_width: resolved_missing_width.unwrap_or(DEFAULT_GLYPH_WIDTH_UNITS),
    })
}

fn cid_width_table(
    document: &Document,
    font_dict: &Dictionary,
    resolved_missing_width: Option<f64>,
) -> Option<CidWidthTable> {
    let descendant = descendant_font(document, font_dict)?;
    // /DW takes precedence per PDF spec; fall back to MissingWidth when /DW is
    // absent, only then fall back to the CID 1000.0 constant.
    let default_width = descendant
        .get(b"DW")
        .ok()
        .and_then(object_to_f64)
        .or(resolved_missing_width)
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

#[cfg(test)]
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
