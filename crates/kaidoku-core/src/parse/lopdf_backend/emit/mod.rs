mod fonts;
mod matrix;
mod state;
#[cfg(test)]
mod tests;
mod text;

use super::resources::ImageMetadata;
use crate::{
    BBox, ElementKind, ExtractError, ImagePayload, PageNumber, RawElement, RawPayload, SourceRef,
};
use fonts::FontCatalog;
use lopdf::{Document, Object, ObjectId, content::Content, content::Operation};
use matrix::Matrix;
use state::{GraphicsState, TextState};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy)]
pub(super) struct OperationCursor {
    next_index: u32,
}

impl OperationCursor {
    pub(super) fn next_element_index(&mut self) -> Result<u32, ExtractError> {
        let current = self.next_index;
        self.next_index =
            self.next_index
                .checked_add(1)
                .ok_or(ExtractError::InvariantViolation {
                    reason: "element index overflow".to_string(),
                })?;
        Ok(current)
    }
}

pub(super) struct EmitContext<'a> {
    page_number: PageNumber,
    page_id: ObjectId,
    stream_index: u32,
    coordinate_precision: u8,
    image_catalog: &'a HashMap<Vec<u8>, ImageMetadata>,
    font_catalog: &'a FontCatalog<'a>,
    max_elements_per_page: u32,
    out: &'a mut Vec<RawElement>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ExtractionLimits {
    pub(super) operation_budget: u32,
    pub(super) max_elements: u32,
    pub(super) stream_byte_limit: usize,
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

    pub(super) fn push_element(
        &mut self,
        operation_index: u32,
        element_index: u32,
        kind: ElementKind,
        bbox: BBox,
        payload: RawPayload,
    ) -> Result<(), ExtractError> {
        let next_count = self
            .out
            .len()
            .checked_add(1)
            .ok_or(ExtractError::InvariantViolation {
                reason: "element count overflow".to_string(),
            })?;
        let next_count_u32 =
            u32::try_from(next_count).map_err(|_| ExtractError::InvariantViolation {
                reason: "element count does not fit into u32".to_string(),
            })?;

        if next_count_u32 > self.max_elements_per_page {
            return Err(ExtractError::ExtractionLimitExceeded {
                page_number: self.page_number.get(),
                kind: "elements_per_page",
                limit: u64::from(self.max_elements_per_page),
                actual: u64::from(next_count_u32),
            });
        }

        let source_ref = self.source_ref(operation_index, element_index)?;
        self.out
            .push(RawElement::new(kind, bbox, payload, source_ref));
        Ok(())
    }
}

pub(super) fn extract_page_elements(
    document: &Document,
    page_number: PageNumber,
    page_id: ObjectId,
    coordinate_precision: u8,
    image_catalog: &HashMap<Vec<u8>, ImageMetadata>,
    limits: ExtractionLimits,
) -> Result<Vec<RawElement>, ExtractError> {
    let mut elements = Vec::new();
    let font_catalog = FontCatalog::from_page(document, page_id)?;
    let mut text_state = TextState::default();
    let mut graphics_state = GraphicsState::default();

    let stream_ids = document.get_page_contents(page_id);
    let mut total_operations: u32 = 0;

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

        if content_bytes.len() > limits.stream_byte_limit {
            return Err(ExtractError::ExtractionLimitExceeded {
                page_number: page_number.get(),
                kind: "content_stream_bytes",
                limit: u64::try_from(limits.stream_byte_limit).map_err(|_| {
                    ExtractError::InvariantViolation {
                        reason: "content stream byte limit does not fit in u64".to_string(),
                    }
                })?,
                actual: u64::try_from(content_bytes.len()).map_err(|_| {
                    ExtractError::InvariantViolation {
                        reason: "content stream byte count does not fit in u64".to_string(),
                    }
                })?,
            });
        }

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
            font_catalog: &font_catalog,
            max_elements_per_page: limits.max_elements,
            out: &mut elements,
        };

        for (op_idx, operation) in content.operations.iter().enumerate() {
            total_operations =
                total_operations
                    .checked_add(1)
                    .ok_or(ExtractError::InvariantViolation {
                        reason: "operation count overflow".to_string(),
                    })?;

            if total_operations > limits.operation_budget {
                return Err(ExtractError::ExtractionLimitExceeded {
                    page_number: page_number.get(),
                    kind: "operations_per_page",
                    limit: u64::from(limits.operation_budget),
                    actual: u64::from(total_operations),
                });
            }

            let operation_index =
                u32::try_from(op_idx).map_err(|_| ExtractError::InvariantViolation {
                    reason: "operation index overflow".to_string(),
                })?;

            let mut cursor = OperationCursor { next_index: 0 };
            process_operation(
                operation,
                operation_index,
                &mut text_state,
                &mut graphics_state,
                &mut cursor,
                &mut emit,
            )?;
        }
    }

    Ok(elements)
}

fn process_operation(
    operation: &Operation,
    operation_index: u32,
    text_state: &mut TextState,
    graphics_state: &mut GraphicsState,
    cursor: &mut OperationCursor,
    context: &mut EmitContext<'_>,
) -> Result<(), ExtractError> {
    if handle_text_state_operation(operation, text_state)? {
        return Ok(());
    }

    if handle_text_show_operation(
        operation,
        operation_index,
        text_state,
        graphics_state,
        cursor,
        context,
    )? {
        return Ok(());
    }

    match operation.operator.as_str() {
        "cm" => {
            let matrix = Matrix::from_operands(&operation.operands)?;
            graphics_state.concatenate_ctm(matrix);
        }
        "q" => graphics_state.save(),
        "Q" => graphics_state.restore(),
        "Do" => {
            if let Some(first) = operation.operands.first() {
                maybe_emit_image_element(
                    first,
                    graphics_state.ctm(),
                    operation_index,
                    cursor,
                    context,
                )?;
            }
        }
        _ => {}
    }

    Ok(())
}

fn handle_text_state_operation(
    operation: &Operation,
    text_state: &mut TextState,
) -> Result<bool, ExtractError> {
    match operation.operator.as_str() {
        "BT" => text_state.begin_text_object(),
        "Tf" => update_text_font(operation, text_state)?,
        "Tc" => {
            if let Some(first) = operation.operands.first() {
                text_state.set_char_spacing(object_to_f64(first)?);
            }
        }
        "Tw" => {
            if let Some(first) = operation.operands.first() {
                text_state.set_word_spacing(object_to_f64(first)?);
            }
        }
        "Tz" => {
            if let Some(first) = operation.operands.first() {
                text_state.set_horizontal_scaling(object_to_f64(first)?);
            }
        }
        "TL" => {
            if let Some(first) = operation.operands.first() {
                text_state.set_leading(object_to_f64(first)?);
            }
        }
        "Ts" => {
            if let Some(first) = operation.operands.first() {
                text_state.set_text_rise(object_to_f64(first)?);
            }
        }
        "Td" | "TD" => update_text_position(operation, text_state)?,
        "Tm" => update_text_matrix(operation, text_state)?,
        "T*" => text_state.next_line(),
        _ => return Ok(false),
    }

    Ok(true)
}

fn handle_text_show_operation(
    operation: &Operation,
    operation_index: u32,
    text_state: &mut TextState,
    graphics_state: &GraphicsState,
    cursor: &mut OperationCursor,
    context: &mut EmitContext<'_>,
) -> Result<bool, ExtractError> {
    match operation.operator.as_str() {
        "'" => {
            text_state.next_line();
            if let Some(first) = operation.operands.first() {
                text::emit_text_elements(
                    first,
                    text_state,
                    graphics_state,
                    operation_index,
                    cursor,
                    context,
                )?;
            }
        }
        "\"" => {
            if operation.operands.len() >= 2 {
                text_state.set_word_spacing(object_to_f64(&operation.operands[0])?);
                text_state.set_char_spacing(object_to_f64(&operation.operands[1])?);
            }
            text_state.next_line();
            if let Some(last) = operation.operands.last() {
                text::emit_text_elements(
                    last,
                    text_state,
                    graphics_state,
                    operation_index,
                    cursor,
                    context,
                )?;
            }
        }
        "Tj" => {
            if let Some(first) = operation.operands.first() {
                text::emit_text_elements(
                    first,
                    text_state,
                    graphics_state,
                    operation_index,
                    cursor,
                    context,
                )?;
            }
        }
        "TJ" => {
            if let Some(first) = operation.operands.first() {
                text::emit_text_array(
                    first,
                    text_state,
                    graphics_state,
                    operation_index,
                    cursor,
                    context,
                )?;
            }
        }
        _ => return Ok(false),
    }

    Ok(true)
}

fn update_text_font(operation: &Operation, text_state: &mut TextState) -> Result<(), ExtractError> {
    if operation.operands.len() >= 2 {
        let key = operation.operands[0].as_name().ok().map(<[u8]>::to_vec);
        let font_size = object_to_f64(&operation.operands[1])?.abs();
        text_state.set_font(key, font_size);
    }
    Ok(())
}

fn update_text_position(
    operation: &Operation,
    text_state: &mut TextState,
) -> Result<(), ExtractError> {
    if operation.operands.len() >= 2 {
        let tx = object_to_f64(&operation.operands[0])?;
        let ty = object_to_f64(&operation.operands[1])?;
        text_state.move_text_position(tx, ty, operation.operator == "TD");
    }
    Ok(())
}

fn update_text_matrix(
    operation: &Operation,
    text_state: &mut TextState,
) -> Result<(), ExtractError> {
    if operation.operands.len() == 6 {
        let matrix = Matrix::from_operands(&operation.operands)?;
        text_state.set_text_matrix(matrix);
    }
    Ok(())
}

fn maybe_emit_image_element(
    name_object: &Object,
    ctm: Matrix,
    operation_index: u32,
    cursor: &mut OperationCursor,
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
    let element_index = cursor.next_element_index()?;
    context.push_element(
        operation_index,
        element_index,
        ElementKind::Image,
        bbox,
        RawPayload::Image(ImagePayload {
            name: metadata.name.clone(),
            width_px: metadata.width_px,
            height_px: metadata.height_px,
            color_space: metadata.color_space.clone(),
            bits_per_component: metadata.bits_per_component,
        }),
    )
}

pub(super) fn object_to_f64(value: &Object) -> Result<f64, ExtractError> {
    value
        .as_float()
        .map(f64::from)
        .map_err(|error| ExtractError::ContentDecode {
            reason: error.to_string(),
        })
}

pub(super) fn element_sort_key(element: &RawElement) -> (u32, u32, u32, u8) {
    (
        element.source_ref().stream_index(),
        element.source_ref().operation_index(),
        element.source_ref().element_index(),
        element_kind_order(element.kind()),
    )
}

const fn element_kind_order(kind: ElementKind) -> u8 {
    match kind {
        ElementKind::Span => 0,
        ElementKind::Char => 1,
        ElementKind::Image => 2,
    }
}
