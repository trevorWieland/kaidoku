use super::resources::ImageMetadata;
use crate::{
    BBox, CharPayload, ElementKind, ExtractError, ImagePayload, PageNumber, RawElement, RawPayload,
    SourceRef, SpanPayload,
};
use lopdf::{Document, Object, ObjectId, content::Content, content::Operation};
use std::collections::HashMap;

#[derive(Debug, Clone)]
struct TextState {
    x: f64,
    y: f64,
    leading: f64,
    font_name: Option<String>,
    font_size: f64,
}

impl Default for TextState {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            leading: 0.0,
            font_name: None,
            font_size: 12.0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Matrix {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    e: f64,
    f: f64,
}

impl Matrix {
    const fn identity() -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        }
    }

    fn from_operands(operands: &[Object]) -> Result<Self, ExtractError> {
        if operands.len() != 6 {
            return Err(ExtractError::InvariantViolation {
                reason: "cm operator requires 6 operands".to_string(),
            });
        }
        Ok(Self {
            a: f64::from(object_to_f32(&operands[0])?),
            b: f64::from(object_to_f32(&operands[1])?),
            c: f64::from(object_to_f32(&operands[2])?),
            d: f64::from(object_to_f32(&operands[3])?),
            e: f64::from(object_to_f32(&operands[4])?),
            f: f64::from(object_to_f32(&operands[5])?),
        })
    }

    #[must_use]
    fn concatenate(self, next: Self) -> Self {
        Self {
            a: (self.a * next.a) + (self.b * next.c),
            b: (self.a * next.b) + (self.b * next.d),
            c: (self.c * next.a) + (self.d * next.c),
            d: (self.c * next.b) + (self.d * next.d),
            e: (self.e * next.a) + (self.f * next.c) + next.e,
            f: (self.e * next.b) + (self.f * next.d) + next.f,
        }
    }

    #[must_use]
    fn to_bbox(self) -> Option<(f64, f64, f64, f64)> {
        let p0 = self.transform(0.0, 0.0);
        let p1 = self.transform(1.0, 0.0);
        let p2 = self.transform(0.0, 1.0);
        let p3 = self.transform(1.0, 1.0);

        let min_x = p0.0.min(p1.0).min(p2.0).min(p3.0);
        let max_x = p0.0.max(p1.0).max(p2.0).max(p3.0);
        let min_y = p0.1.min(p1.1).min(p2.1).min(p3.1);
        let max_y = p0.1.max(p1.1).max(p2.1).max(p3.1);

        let width = (max_x - min_x).abs();
        let height = (max_y - min_y).abs();
        if width == 0.0 || height == 0.0 {
            return None;
        }
        Some((min_x, min_y, width, height))
    }

    #[must_use]
    fn transform(self, x: f64, y: f64) -> (f64, f64) {
        (
            (self.a * x) + (self.c * y) + self.e,
            (self.b * x) + (self.d * y) + self.f,
        )
    }
}

struct EmitContext<'a> {
    page_number: PageNumber,
    page_id: ObjectId,
    stream_index: u32,
    coordinate_precision: u8,
    image_catalog: &'a HashMap<Vec<u8>, ImageMetadata>,
    out: &'a mut Vec<RawElement>,
}

impl EmitContext<'_> {
    fn source_ref(
        &self,
        operation_index: u32,
        element_index: u32,
    ) -> Result<SourceRef, ExtractError> {
        SourceRef::new(
            self.page_number.get(),
            self.page_id.0,
            self.page_id.1,
            self.stream_index,
            operation_index,
            element_index,
        )
        .map_err(ExtractError::from)
    }
}

pub(super) fn extract_page_elements(
    document: &Document,
    page_number: PageNumber,
    page_id: ObjectId,
    coordinate_precision: u8,
    image_catalog: &HashMap<Vec<u8>, ImageMetadata>,
) -> Result<Vec<RawElement>, ExtractError> {
    let mut elements = Vec::new();

    let stream_ids = document.get_page_contents(page_id);
    for (stream_idx, stream_id) in stream_ids.iter().enumerate() {
        let stream = document
            .get_object(*stream_id)
            .and_then(Object::as_stream)
            .map_err(|error| ExtractError::ContentDecode {
                reason: error.to_string(),
            })?;

        let content_bytes = match stream.decompressed_content() {
            Ok(content) => content,
            Err(_) => stream.content.clone(),
        };

        let content =
            Content::decode(&content_bytes).map_err(|error| ExtractError::ContentDecode {
                reason: error.to_string(),
            })?;

        let stream_index =
            u32::try_from(stream_idx).map_err(|_| ExtractError::InvariantViolation {
                reason: "stream index overflow".to_string(),
            })?;

        let mut emit = EmitContext {
            page_number,
            page_id,
            stream_index,
            coordinate_precision,
            image_catalog,
            out: &mut elements,
        };

        extract_content_operations(&content, &mut emit)?;
    }

    Ok(elements)
}

fn extract_content_operations(
    content: &Content,
    emit_ctx: &mut EmitContext<'_>,
) -> Result<(), ExtractError> {
    let mut text_state = TextState::default();
    let mut ctm = Matrix::identity();
    let mut stack = Vec::new();

    for (op_idx, operation) in content.operations.iter().enumerate() {
        let operation_index =
            u32::try_from(op_idx).map_err(|_| ExtractError::InvariantViolation {
                reason: "operation index overflow".to_string(),
            })?;

        process_operation(
            operation,
            operation_index,
            &mut text_state,
            &mut ctm,
            &mut stack,
            emit_ctx,
        )?;
    }

    Ok(())
}

fn process_operation(
    operation: &Operation,
    operation_index: u32,
    text_state: &mut TextState,
    ctm: &mut Matrix,
    stack: &mut Vec<Matrix>,
    context: &mut EmitContext<'_>,
) -> Result<(), ExtractError> {
    match operation.operator.as_str() {
        "BT" => {
            *text_state = TextState::default();
        }
        "Tf" => update_text_font(operation, text_state)?,
        "Td" | "TD" => update_text_position(operation, text_state)?,
        "Tm" => update_text_matrix(operation, text_state)?,
        "T*" => {
            text_state.y -= text_state.leading;
        }
        "'" => {
            text_state.y -= text_state.leading;
            if let Some(first) = operation.operands.first() {
                emit_text_elements(first, text_state, operation_index, context)?;
            }
        }
        "\"" => {
            text_state.y -= text_state.leading;
            if let Some(last) = operation.operands.last() {
                emit_text_elements(last, text_state, operation_index, context)?;
            }
        }
        "Tj" => {
            if let Some(first) = operation.operands.first() {
                emit_text_elements(first, text_state, operation_index, context)?;
            }
        }
        "TJ" => {
            if let Some(first) = operation.operands.first() {
                emit_text_array(first, text_state, operation_index, context)?;
            }
        }
        "cm" => {
            let matrix = Matrix::from_operands(&operation.operands)?;
            *ctm = ctm.concatenate(matrix);
        }
        "q" => stack.push(*ctm),
        "Q" => {
            if let Some(previous) = stack.pop() {
                *ctm = previous;
            }
        }
        "Do" => {
            if let Some(first) = operation.operands.first() {
                maybe_emit_image_element(first, *ctm, operation_index, context)?;
            }
        }
        _ => {}
    }

    Ok(())
}

fn update_text_font(operation: &Operation, text_state: &mut TextState) -> Result<(), ExtractError> {
    if operation.operands.len() >= 2 {
        text_state.font_name = extract_font_name(&operation.operands[0]);
        text_state.font_size = f64::from(object_to_f32(&operation.operands[1])?).abs();
    }
    Ok(())
}

fn update_text_position(
    operation: &Operation,
    text_state: &mut TextState,
) -> Result<(), ExtractError> {
    if operation.operands.len() >= 2 {
        let tx = f64::from(object_to_f32(&operation.operands[0])?);
        let ty = f64::from(object_to_f32(&operation.operands[1])?);
        text_state.x += tx;
        text_state.y += ty;
        if operation.operator == "TD" {
            text_state.leading = -ty;
        }
    }
    Ok(())
}

fn update_text_matrix(
    operation: &Operation,
    text_state: &mut TextState,
) -> Result<(), ExtractError> {
    if operation.operands.len() == 6 {
        text_state.x = f64::from(object_to_f32(&operation.operands[4])?);
        text_state.y = f64::from(object_to_f32(&operation.operands[5])?);
    }
    Ok(())
}

fn maybe_emit_image_element(
    name_object: &Object,
    ctm: Matrix,
    operation_index: u32,
    context: &mut EmitContext<'_>,
) -> Result<(), ExtractError> {
    let name = name_object
        .as_name()
        .map_err(|error| ExtractError::ContentDecode {
            reason: error.to_string(),
        })?;

    let Some(metadata) = context.image_catalog.get(name) else {
        return Ok(());
    };

    let (x, y, width, height) = if let Some(bbox) = ctm.to_bbox() {
        bbox
    } else {
        (
            0.0,
            0.0,
            f64::from(metadata.width_px),
            f64::from(metadata.height_px),
        )
    };

    let bbox = BBox::new(x, y, width, height)?.quantized(context.coordinate_precision);
    context.out.push(RawElement {
        kind: ElementKind::Image,
        bbox,
        payload: RawPayload::Image(ImagePayload {
            name: metadata.name.clone(),
            width_px: metadata.width_px,
            height_px: metadata.height_px,
            color_space: metadata.color_space.clone(),
            bits_per_component: metadata.bits_per_component,
        }),
        source_ref: context.source_ref(operation_index, 0)?,
    });

    Ok(())
}

fn emit_text_array(
    array_object: &Object,
    text_state: &mut TextState,
    operation_index: u32,
    context: &mut EmitContext<'_>,
) -> Result<(), ExtractError> {
    let items = array_object
        .as_array()
        .map_err(|error| ExtractError::ContentDecode {
            reason: error.to_string(),
        })?;

    for item in items {
        if matches!(item, Object::String(_, _)) {
            emit_text_elements(item, text_state, operation_index, context)?;
            continue;
        }

        if let Ok(adjustment) = item.as_float() {
            let spacing = f64::from(adjustment) * text_state.font_size / 1000.0;
            text_state.x -= spacing;
        }
    }

    Ok(())
}

fn emit_text_elements(
    text_object: &Object,
    text_state: &mut TextState,
    operation_index: u32,
    context: &mut EmitContext<'_>,
) -> Result<(), ExtractError> {
    let Some(text) = decode_text(text_object) else {
        return Ok(());
    };
    if text.is_empty() {
        return Ok(());
    }

    let char_count =
        u32::try_from(text.chars().count()).map_err(|_| ExtractError::InvariantViolation {
            reason: "character count overflow".to_string(),
        })?;
    let span_width = estimate_text_width(char_count, text_state.font_size);

    let span_bbox = BBox::new(
        text_state.x,
        text_state.y - text_state.font_size,
        span_width,
        text_state.font_size,
    )?
    .quantized(context.coordinate_precision);

    context.out.push(RawElement {
        kind: ElementKind::Span,
        bbox: span_bbox,
        payload: RawPayload::Span(SpanPayload {
            text: text.clone(),
            font_name: text_state.font_name.clone(),
            font_size: text_state.font_size,
        }),
        source_ref: context.source_ref(operation_index, 0)?,
    });

    let per_char_width = if char_count == 0 {
        0.0
    } else {
        span_width / f64::from(char_count)
    };

    for (char_idx, character) in text.chars().enumerate() {
        let idx_u32 = u32::try_from(char_idx).map_err(|_| ExtractError::InvariantViolation {
            reason: "character index overflow".to_string(),
        })?;

        let bbox = BBox::new(
            text_state.x + (f64::from(idx_u32) * per_char_width),
            text_state.y - text_state.font_size,
            per_char_width,
            text_state.font_size,
        )?
        .quantized(context.coordinate_precision);

        context.out.push(RawElement {
            kind: ElementKind::Char,
            bbox,
            payload: RawPayload::Char(CharPayload {
                text: character.to_string(),
                font_name: text_state.font_name.clone(),
                font_size: text_state.font_size,
                char_index: idx_u32,
            }),
            source_ref: context.source_ref(operation_index, idx_u32 + 1)?,
        });
    }

    text_state.x += span_width;
    Ok(())
}

fn extract_font_name(object: &Object) -> Option<String> {
    object
        .as_name()
        .ok()
        .map(|bytes| String::from_utf8_lossy(bytes).to_string())
}

fn decode_text(object: &Object) -> Option<String> {
    object
        .as_str()
        .ok()
        .map(|bytes| String::from_utf8_lossy(bytes).to_string())
}

fn estimate_text_width(char_count: u32, font_size: f64) -> f64 {
    f64::from(char_count) * font_size * 0.5
}

fn object_to_f32(value: &Object) -> Result<f32, ExtractError> {
    value
        .as_float()
        .map_err(|error| ExtractError::ContentDecode {
            reason: error.to_string(),
        })
}

pub(super) fn element_sort_key(element: &RawElement) -> (u32, u32, u32, u8) {
    (
        element.source_ref.stream_index,
        element.source_ref.operation_index,
        element.source_ref.element_index,
        element_kind_order(&element.kind),
    )
}

const fn element_kind_order(kind: &ElementKind) -> u8 {
    match kind {
        ElementKind::Span => 0,
        ElementKind::Char => 1,
        ElementKind::Image => 2,
    }
}
