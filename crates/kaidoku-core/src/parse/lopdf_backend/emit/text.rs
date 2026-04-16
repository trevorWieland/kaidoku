use super::state::{GraphicsState, TextState};
use super::{EmitContext, OperationCursor};
use crate::{BBox, CharPayload, ExtractError, SpanPayload};
use lopdf::Object;

pub(super) fn emit_text_array(
    array_object: &Object,
    text_state: &mut TextState,
    graphics_state: &GraphicsState,
    operation_index: u32,
    cursor: &mut OperationCursor,
    context: &mut EmitContext<'_>,
) -> Result<(), ExtractError> {
    let items = array_object
        .as_array()
        .map_err(|error| ExtractError::ContentDecode {
            reason: error.to_string(),
        })?;

    for item in items {
        if matches!(item, Object::String(_, _)) {
            emit_text_elements(
                item,
                text_state,
                graphics_state,
                operation_index,
                cursor,
                context,
            )?;
            continue;
        }

        if let Ok(adjustment) = item.as_float() {
            text_state.apply_tj_adjustment(f64::from(adjustment));
        }
    }

    Ok(())
}

struct PlacedGlyph {
    character: char,
    bbox: BBox,
}

pub(super) fn emit_text_elements(
    text_object: &Object,
    text_state: &mut TextState,
    graphics_state: &GraphicsState,
    operation_index: u32,
    cursor: &mut OperationCursor,
    context: &mut EmitContext<'_>,
) -> Result<(), ExtractError> {
    let raw_bytes = match text_object {
        Object::String(bytes, _) => bytes.as_slice(),
        _ => return Ok(()),
    };

    let glyph_runs = context
        .font_catalog
        .glyph_runs(text_state.font_key(), raw_bytes);
    if glyph_runs.is_empty() {
        return Ok(());
    }
    let decoded_text = glyph_runs
        .iter()
        .map(|run| run.text.as_str())
        .collect::<String>();
    if decoded_text.is_empty() {
        return Ok(());
    }
    let char_count = glyph_runs
        .iter()
        .map(|run| run.text.chars().count())
        .sum::<usize>();
    if char_count == 0 {
        return Ok(());
    }

    let font_id = context.intern_font_id(text_state.font_key())?;

    // Single pass over glyph runs: compute each glyph's bbox once, union it
    // into the span's bbox, and retain the per-glyph result so the char
    // emission loop below can reuse it without recomputing advances or
    // transforms. This collapses the previous probe/emit dual pass.
    let mut placed: Vec<PlacedGlyph> = Vec::with_capacity(char_count);
    let mut span_bbox: Option<BBox> = None;
    for run in &glyph_runs {
        let advances = glyph_char_advances(text_state, run.text.as_str(), run.width_units.max(0.0));
        for (character, glyph_advance, total_advance) in advances {
            let bbox = char_bbox_for_state(text_state, graphics_state, glyph_advance)?;
            span_bbox = Some(match span_bbox {
                None => bbox,
                Some(previous) => merge_bbox(previous, bbox)?,
            });
            placed.push(PlacedGlyph { character, bbox });
            text_state.advance_text(total_advance);
        }
    }

    let span_bbox = if let Some(span_bbox) = span_bbox {
        span_bbox
    } else {
        let origin = text_state.current_origin(graphics_state.ctm());
        BBox::new(origin.0, origin.1, 1e-6, text_state.font_size()).map_err(|error| {
            ExtractError::InvalidFallbackGeometry {
                page_number: context.page_number().get(),
                reason: format!("failed constructing span fallback bbox: {error}"),
            }
        })?
    };

    let span_element_index = cursor.next_element_index()?;
    context.push_span(
        operation_index,
        span_element_index,
        span_bbox,
        SpanPayload::new(decoded_text, font_id, text_state.font_size())?,
    )?;

    for (char_index_usize, glyph) in placed.into_iter().enumerate() {
        let char_index =
            u32::try_from(char_index_usize).map_err(|_| ExtractError::InvariantViolation {
                reason: "character index overflow".to_string(),
            })?;
        let char_element_index = cursor.next_element_index()?;
        context.push_char(
            operation_index,
            char_element_index,
            glyph.bbox,
            CharPayload::new(
                glyph.character.to_string(),
                font_id,
                text_state.font_size(),
                char_index,
            )?,
        )?;
    }

    Ok(())
}

fn glyph_char_advances(
    text_state: &TextState,
    glyph_text: &str,
    glyph_width_units: f64,
) -> Vec<(char, f64, f64)> {
    let chars = glyph_text.chars().collect::<Vec<char>>();
    if chars.is_empty() {
        return Vec::new();
    }

    let glyph_advance = (glyph_width_units / 1000.0)
        * text_state.font_size()
        * text_state.horizontal_scale_factor();
    let spacing_advance = glyph_spacing_advance(text_state, &chars);
    let chars_len = chars.len();
    let per_char_advance = if chars_len == 1 {
        glyph_advance.max(0.0)
    } else {
        let chars_len_f64 = u32::try_from(chars_len).ok().map_or(1.0, f64::from);
        (glyph_advance / chars_len_f64).max(0.0)
    };

    let mut out = Vec::with_capacity(chars_len);
    for (index, character) in chars.into_iter().enumerate() {
        let is_last = index + 1 == chars_len;
        let total_advance = if is_last {
            (per_char_advance + spacing_advance).max(0.0)
        } else {
            per_char_advance.max(0.0)
        };
        out.push((character, per_char_advance, total_advance));
    }
    out
}

fn glyph_spacing_advance(text_state: &TextState, glyph_chars: &[char]) -> f64 {
    let is_space = glyph_chars.len() == 1 && glyph_chars[0] == ' ';
    (text_state.char_spacing()
        + if is_space {
            text_state.word_spacing()
        } else {
            0.0
        })
        * text_state.horizontal_scale_factor()
}

fn char_bbox_for_state(
    text_state: &TextState,
    graphics_state: &GraphicsState,
    glyph_advance: f64,
) -> Result<BBox, ExtractError> {
    if let Some((x, y, width, height)) = text_state
        .glyph_transform(graphics_state.ctm(), glyph_advance.max(1e-6))
        .to_bbox()
    {
        BBox::new(x, y, width, height).map_err(ExtractError::from)
    } else {
        let origin = text_state.current_origin(graphics_state.ctm());
        BBox::new(
            origin.0,
            origin.1,
            glyph_advance.max(1e-6),
            text_state.font_size(),
        )
        .map_err(ExtractError::from)
    }
}

fn merge_bbox(left: BBox, right: BBox) -> Result<BBox, ExtractError> {
    let min_x = left.x().min(right.x());
    let min_y = left.y().min(right.y());
    let max_x = (left.x() + left.width()).max(right.x() + right.width());
    let max_y = (left.y() + left.height()).max(right.y() + right.height());

    BBox::new(min_x, min_y, max_x - min_x, max_y - min_y).map_err(ExtractError::from)
}
