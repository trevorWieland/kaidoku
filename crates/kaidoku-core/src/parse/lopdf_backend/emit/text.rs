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

    let decoded_text = context
        .font_catalog
        .decode_text(text_state.font_key(), raw_bytes);
    if decoded_text.is_empty() {
        return Ok(());
    }

    let char_count = decoded_text.chars().count();
    if char_count == 0 {
        return Ok(());
    }

    let mut width_units =
        context
            .font_catalog
            .glyph_widths(text_state.font_key(), raw_bytes, char_count);
    width_units = align_widths_with_char_count(width_units, char_count);

    let font_id = context.intern_font_id(text_state.font_key())?;

    let mut probe_state = text_state.clone();
    let mut span_bbox: Option<BBox> = None;
    for (idx, character) in decoded_text.chars().enumerate() {
        let glyph_width_units = width_units.get(idx).copied().unwrap_or(500.0).max(0.0);
        let (glyph_advance, total_advance) = advances(&probe_state, character, glyph_width_units);

        let char_bbox = char_bbox_for_state(&probe_state, graphics_state, glyph_advance)?;
        span_bbox = Some(match span_bbox {
            None => char_bbox,
            Some(previous) => merge_bbox(previous, char_bbox)?,
        });

        probe_state.advance_text(total_advance);
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
        SpanPayload {
            text: decoded_text.clone(),
            font_id,
            font_size: text_state.font_size(),
        },
    )?;

    for (idx, character) in decoded_text.chars().enumerate() {
        let idx_u32 = u32::try_from(idx).map_err(|_| ExtractError::InvariantViolation {
            reason: "character index overflow".to_string(),
        })?;

        let glyph_width_units = width_units.get(idx).copied().unwrap_or(500.0).max(0.0);
        let (glyph_advance, total_advance) = advances(text_state, character, glyph_width_units);

        let char_bbox = char_bbox_for_state(text_state, graphics_state, glyph_advance)?;

        let char_element_index = cursor.next_element_index()?;
        context.push_char(
            operation_index,
            char_element_index,
            char_bbox,
            CharPayload {
                text: character.to_string(),
                font_id,
                font_size: text_state.font_size(),
                char_index: idx_u32,
            },
        )?;

        text_state.advance_text(total_advance);
    }

    Ok(())
}

fn advances(text_state: &TextState, character: char, glyph_width_units: f64) -> (f64, f64) {
    let glyph_advance = (glyph_width_units / 1000.0)
        * text_state.font_size()
        * text_state.horizontal_scale_factor();
    let spacing_advance = (text_state.char_spacing()
        + if character == ' ' {
            text_state.word_spacing()
        } else {
            0.0
        })
        * text_state.horizontal_scale_factor();

    let total_advance = (glyph_advance + spacing_advance).max(0.0);
    (glyph_advance, total_advance)
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

fn align_widths_with_char_count(mut widths: Vec<f64>, char_count: usize) -> Vec<f64> {
    if char_count == 0 {
        return Vec::new();
    }

    if widths.len() == char_count {
        return widths;
    }

    if widths.is_empty() {
        return vec![500.0; char_count];
    }

    let total = widths.iter().copied().sum::<f64>();
    let char_count_f64 = u32::try_from(char_count).ok().map(f64::from);
    let per_char = if total <= 0.0 {
        500.0
    } else if let Some(char_count_f64) = char_count_f64 {
        total / char_count_f64
    } else {
        500.0
    };

    widths.clear();
    widths.resize(char_count, per_char);
    widths
}
