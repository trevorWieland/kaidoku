use super::matrix::Matrix;
use super::resources::ResourceScope;
use super::state::{GraphicsState, TextState};
use super::{
    EmitContext, OperationCursor, ProcessRuntime, object_to_f64, process_stream, resources, text,
};
use crate::{BBox, ExtractError, ImagePayload};
use lopdf::{Object, Stream, content::Operation};

pub(super) fn process_operation(
    operation: &Operation,
    operation_index: u32,
    text_state: &mut TextState,
    graphics_state: &mut GraphicsState,
    cursor: &mut OperationCursor,
    context: &mut EmitContext<'_>,
    shared: (&ResourceScope, &mut ProcessRuntime<'_>),
) -> Result<(), ExtractError> {
    let (scope, runtime) = shared;
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
        "BI" => {
            maybe_emit_inline_image_element(
                operation,
                graphics_state.ctm(),
                operation_index,
                cursor,
                context,
            )?;
        }
        "Do" => {
            if let Some(first) = operation.operands.first() {
                handle_xobject_invocation(
                    first,
                    scope,
                    graphics_state.ctm(),
                    operation_index,
                    cursor,
                    context,
                    runtime,
                )?;
            }
        }
        _ => {}
    }

    Ok(())
}

fn handle_xobject_invocation(
    name_object: &Object,
    scope: &ResourceScope,
    ctm: Matrix,
    operation_index: u32,
    cursor: &mut OperationCursor,
    context: &mut EmitContext<'_>,
    runtime: &mut ProcessRuntime<'_>,
) -> Result<(), ExtractError> {
    let name = name_object
        .as_name()
        .map_err(|error| ExtractError::ContentDecode {
            reason: error.to_string(),
        })?;

    let Some(object_id) = scope.resolve_xobject(name) else {
        return Ok(());
    };

    let object =
        runtime
            .document
            .get_object(object_id)
            .map_err(|error| ExtractError::ContentDecode {
                reason: error.to_string(),
            })?;
    let stream = object
        .as_stream()
        .map_err(|error| ExtractError::ContentDecode {
            reason: error.to_string(),
        })?;

    let subtype = stream
        .dict
        .get(b"Subtype")
        .and_then(Object::as_name)
        .map_err(|error| ExtractError::ContentDecode {
            reason: error.to_string(),
        })?;

    if subtype == b"Image" {
        if let Some(metadata) =
            resources::image_metadata_for_object(runtime.document, object_id, name)
        {
            let (x, y, width, height) = ctm.to_bbox().unwrap_or((
                0.0,
                0.0,
                f64::from(metadata.width_px),
                f64::from(metadata.height_px),
            ));

            let bbox = BBox::new(x, y, width, height)?;
            let element_index = cursor.next_element_index()?;
            return context.push_image(
                operation_index,
                element_index,
                bbox,
                ImagePayload {
                    name: metadata.name,
                    width_px: metadata.width_px,
                    height_px: metadata.height_px,
                    color_space: metadata.color_space,
                    bits_per_component: metadata.bits_per_component,
                },
            );
        }
        return Ok(());
    }

    if subtype != b"Form" {
        return Ok(());
    }

    runtime
        .control
        .checkpoint("form_xobject_enter", Some(context.page_number()))?;
    runtime.traversal.enter(object_id)?;
    let child_scope = resources::form_resource_scope(
        runtime.document,
        scope,
        object_id,
        stream,
        runtime.form_scope_cache,
    );
    let form_matrix =
        resources::form_matrix(stream).map_or_else(Matrix::identity, Matrix::from_values);
    let mut form_text_state = TextState::default();
    let mut form_graphics_state = GraphicsState::with_ctm(ctm.concatenate(form_matrix));

    let recurse_result = process_stream(
        stream,
        &child_scope,
        &mut form_text_state,
        &mut form_graphics_state,
        context,
        runtime,
    );

    runtime.traversal.leave();
    recurse_result
}

fn maybe_emit_inline_image_element(
    operation: &Operation,
    ctm: Matrix,
    operation_index: u32,
    cursor: &mut OperationCursor,
    context: &mut EmitContext<'_>,
) -> Result<(), ExtractError> {
    let Some(first) = operation.operands.first() else {
        return Ok(());
    };

    let Object::Stream(stream) = first else {
        return Ok(());
    };

    let Some((width_px, height_px, color_space, bits_per_component)) =
        inline_image_dimensions(stream)
    else {
        return Ok(());
    };

    let (x, y, width, height) =
        ctm.to_bbox()
            .unwrap_or((0.0, 0.0, f64::from(width_px), f64::from(height_px)));
    let bbox = BBox::new(x, y, width, height)?;
    let element_index = cursor.next_element_index()?;

    let name = format!(
        "inline_{}_{}_{}",
        context.stream_index, operation_index, element_index
    );

    context.push_image(
        operation_index,
        element_index,
        bbox,
        ImagePayload {
            name,
            width_px,
            height_px,
            color_space,
            bits_per_component,
        },
    )
}

fn inline_image_dimensions(stream: &Stream) -> Option<(u32, u32, Option<String>, Option<u8>)> {
    let width_px = stream
        .dict
        .get(b"W")
        .or_else(|_| stream.dict.get(b"Width"))
        .ok()
        .and_then(|value| value.as_i64().ok())
        .and_then(|value| u32::try_from(value).ok())?;

    let height_px = stream
        .dict
        .get(b"H")
        .or_else(|_| stream.dict.get(b"Height"))
        .ok()
        .and_then(|value| value.as_i64().ok())
        .and_then(|value| u32::try_from(value).ok())?;

    let color_space = stream
        .dict
        .get(b"CS")
        .or_else(|_| stream.dict.get(b"ColorSpace"))
        .ok()
        .and_then(extract_color_space);

    let bits_per_component = stream
        .dict
        .get(b"BPC")
        .or_else(|_| stream.dict.get(b"BitsPerComponent"))
        .ok()
        .and_then(|value| value.as_i64().ok())
        .and_then(|value| u8::try_from(value).ok());

    Some((width_px, height_px, color_space, bits_per_component))
}

fn extract_color_space(value: &Object) -> Option<String> {
    match value {
        Object::Name(name) => Some(String::from_utf8_lossy(name).to_string()),
        Object::Array(array) => array
            .first()
            .and_then(|first| first.as_name().ok())
            .map(|name| String::from_utf8_lossy(name).to_string()),
        _ => None,
    }
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
