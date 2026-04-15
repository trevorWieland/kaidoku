mod control;
pub(crate) mod decode;
mod fonts;
mod matrix;
mod ops;
mod state;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_limits;
mod text;

use super::resources::{self, PageGeometry, ResourceScope};
use crate::{BBox, ExtractError, FontId, ImagePayload, PageNumber, RawElement, SourceRef};
use fonts::FontCatalog;
use lopdf::{Document, Object, ObjectId, Stream, content::Content};
use state::{GraphicsState, TextState};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub(crate) use control::ExtractionControl;
pub(super) use control::FontRegistry;

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
    page_geometry: PageGeometry,
    font_catalog: &'a FontCatalog<'a>,
    font_registry: &'a mut FontRegistry,
    max_elements_per_page: u32,
    out: &'a mut Vec<RawElement>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ExtractionLimits {
    pub(super) operation_budget: u32,
    pub(super) max_elements: u32,
    pub(super) stream_byte_limit: usize,
    pub(super) total_stream_budget: usize,
    pub(super) max_form_depth: usize,
    pub(super) max_form_visits: usize,
}

#[derive(Clone, Copy)]
pub(super) struct PageEmitConfig<'a> {
    pub(super) coordinate_precision: u8,
    pub(super) page_geometry: PageGeometry,
    pub(super) root_scope: &'a ResourceScope,
    pub(super) limits: ExtractionLimits,
    pub(super) control: &'a ExtractionControl,
}

#[derive(Debug)]
struct FormTraversal {
    page_number: PageNumber,
    max_depth: usize,
    max_visits: usize,
    stack: Vec<ObjectId>,
    visited: HashSet<ObjectId>,
}

struct ProcessRuntime<'a> {
    document: &'a Document,
    limits: ExtractionLimits,
    total_operations: &'a mut u32,
    next_stream_index: &'a mut u32,
    traversal: &'a mut FormTraversal,
    remaining_decoded_budget: &'a mut usize,
    control: &'a ExtractionControl,
    form_scope_cache: &'a mut HashMap<ObjectId, Arc<HashMap<Vec<u8>, ObjectId>>>,
}

impl FormTraversal {
    fn new(page_number: PageNumber, max_depth: usize, max_visits: usize) -> Self {
        Self {
            page_number,
            max_depth,
            max_visits,
            stack: Vec::new(),
            visited: HashSet::new(),
        }
    }

    fn enter(&mut self, object_id: ObjectId) -> Result<(), ExtractError> {
        if self.stack.len() >= self.max_depth {
            return Err(ExtractError::FormXObjectDepthExceeded {
                page_number: self.page_number.get(),
                depth: self.stack.len().saturating_add(1),
                limit: self.max_depth,
            });
        }

        if self.stack.contains(&object_id) {
            return Err(ExtractError::FormXObjectCycleDetected {
                page_number: self.page_number.get(),
                object_number: object_id.0,
                object_generation: object_id.1,
            });
        }

        self.visited.insert(object_id);
        if self.visited.len() > self.max_visits {
            return Err(ExtractError::FormXObjectVisitLimitExceeded {
                page_number: self.page_number.get(),
                limit: self.max_visits,
                actual: self.visited.len(),
            });
        }

        self.stack.push(object_id);
        Ok(())
    }

    fn leave(&mut self) {
        let _ = self.stack.pop();
    }
}

impl EmitContext<'_> {
    pub(super) const fn page_number(&self) -> PageNumber {
        self.page_number
    }

    pub(super) fn intern_font_id(
        &mut self,
        font_key: Option<&[u8]>,
    ) -> Result<Option<FontId>, ExtractError> {
        let Some(font_name) = self.font_catalog.display_name(font_key) else {
            return Ok(None);
        };
        Ok(Some(self.font_registry.intern(font_name)?))
    }

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

    fn push_element(
        &mut self,
        operation_index: u32,
        element_index: u32,
        bbox: BBox,
        build: impl FnOnce(BBox, SourceRef) -> RawElement,
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
        let normalized_bbox =
            self.page_geometry
                .normalize_bbox(bbox, self.coordinate_precision, self.page_number)?;
        self.out.push(build(normalized_bbox, source_ref));
        Ok(())
    }

    pub(super) fn push_char(
        &mut self,
        operation_index: u32,
        element_index: u32,
        bbox: BBox,
        payload: crate::CharPayload,
    ) -> Result<(), ExtractError> {
        self.push_element(
            operation_index,
            element_index,
            bbox,
            |normalized_bbox, source_ref| RawElement::char(normalized_bbox, source_ref, payload),
        )
    }

    pub(super) fn push_span(
        &mut self,
        operation_index: u32,
        element_index: u32,
        bbox: BBox,
        payload: crate::SpanPayload,
    ) -> Result<(), ExtractError> {
        self.push_element(
            operation_index,
            element_index,
            bbox,
            |normalized_bbox, source_ref| RawElement::span(normalized_bbox, source_ref, payload),
        )
    }

    fn push_image(
        &mut self,
        operation_index: u32,
        element_index: u32,
        bbox: BBox,
        payload: ImagePayload,
    ) -> Result<(), ExtractError> {
        self.push_element(
            operation_index,
            element_index,
            bbox,
            |normalized_bbox, source_ref| RawElement::image(normalized_bbox, source_ref, payload),
        )
    }
}

pub(super) fn extract_page_elements(
    document: &Document,
    page_number: PageNumber,
    page_id: ObjectId,
    config: PageEmitConfig<'_>,
    font_registry: &mut FontRegistry,
    remaining_decoded_budget: &mut usize,
) -> Result<Vec<RawElement>, ExtractError> {
    config
        .control
        .checkpoint("extract_page_elements_start", Some(page_number))?;

    let mut elements = Vec::new();
    let font_catalog = FontCatalog::from_page(document, page_id)?;

    let mut emit = EmitContext {
        page_number,
        page_id,
        stream_index: 0,
        coordinate_precision: config.coordinate_precision,
        page_geometry: config.page_geometry,
        font_catalog: &font_catalog,
        font_registry,
        max_elements_per_page: config.limits.max_elements,
        out: &mut elements,
    };

    let stream_ids = document.get_page_contents(page_id);
    let mut total_operations: u32 = 0;
    let mut next_stream_index: u32 = 0;
    let mut traversal = FormTraversal::new(
        page_number,
        config.limits.max_form_depth,
        config.limits.max_form_visits,
    );
    let mut form_scope_cache: HashMap<ObjectId, Arc<HashMap<Vec<u8>, ObjectId>>> = HashMap::new();

    let mut page_text_state = TextState::default();
    let mut page_graphics_state = GraphicsState::default();

    let mut runtime = ProcessRuntime {
        document,
        limits: config.limits,
        total_operations: &mut total_operations,
        next_stream_index: &mut next_stream_index,
        traversal: &mut traversal,
        remaining_decoded_budget,
        control: config.control,
        form_scope_cache: &mut form_scope_cache,
    };

    for stream_id in &stream_ids {
        runtime
            .control
            .checkpoint("extract_page_stream", Some(page_number))?;

        let stream = runtime
            .document
            .get_object(*stream_id)
            .and_then(Object::as_stream)
            .map_err(|error| ExtractError::ContentDecode {
                reason: error.to_string(),
            })?;

        process_stream(
            stream,
            config.root_scope,
            &mut page_text_state,
            &mut page_graphics_state,
            &mut emit,
            &mut runtime,
        )?;
    }

    Ok(elements)
}

fn process_stream(
    stream: &Stream,
    scope: &ResourceScope,
    text_state: &mut TextState,
    graphics_state: &mut GraphicsState,
    emit: &mut EmitContext<'_>,
    runtime: &mut ProcessRuntime<'_>,
) -> Result<(), ExtractError> {
    let stream_index = allocate_stream_index(runtime.next_stream_index)?;
    let previous_stream_index = emit.stream_index;
    emit.stream_index = stream_index;

    runtime
        .control
        .checkpoint("process_stream_decode", Some(emit.page_number()))?;

    let content_bytes = decode::decode_content_stream_bounded(
        stream,
        emit.page_number,
        stream_index,
        runtime.limits.stream_byte_limit,
        runtime.control,
    )?;

    if content_bytes.len() > *runtime.remaining_decoded_budget {
        let consumed_before = runtime
            .limits
            .total_stream_budget
            .saturating_sub(*runtime.remaining_decoded_budget);
        let actual_bytes = consumed_before.saturating_add(content_bytes.len());
        emit.stream_index = previous_stream_index;
        return Err(ExtractError::DecodedStreamBudgetExceeded {
            page_number: emit.page_number.get(),
            limit_bytes: runtime.limits.total_stream_budget,
            actual_bytes,
        });
    }
    *runtime.remaining_decoded_budget = runtime
        .remaining_decoded_budget
        .saturating_sub(content_bytes.len());

    let content = Content::decode(&content_bytes).map_err(|error| ExtractError::ContentDecode {
        reason: error.to_string(),
    })?;

    for (op_idx, operation) in content.operations.iter().enumerate() {
        runtime
            .control
            .checkpoint("process_stream_operation", Some(emit.page_number()))?;

        *runtime.total_operations =
            runtime
                .total_operations
                .checked_add(1)
                .ok_or(ExtractError::InvariantViolation {
                    reason: "operation count overflow".to_string(),
                })?;

        if *runtime.total_operations > runtime.limits.operation_budget {
            emit.stream_index = previous_stream_index;
            return Err(ExtractError::ExtractionLimitExceeded {
                page_number: emit.page_number.get(),
                kind: "operations_per_page",
                limit: u64::from(runtime.limits.operation_budget),
                actual: u64::from(*runtime.total_operations),
            });
        }

        let operation_index =
            u32::try_from(op_idx).map_err(|_| ExtractError::InvariantViolation {
                reason: "operation index overflow".to_string(),
            })?;

        let mut cursor = OperationCursor { next_index: 0 };
        ops::process_operation(
            operation,
            operation_index,
            text_state,
            graphics_state,
            &mut cursor,
            emit,
            (scope, runtime),
        )?;
    }

    emit.stream_index = previous_stream_index;
    Ok(())
}

fn allocate_stream_index(next_stream_index: &mut u32) -> Result<u32, ExtractError> {
    let current = *next_stream_index;
    *next_stream_index =
        next_stream_index
            .checked_add(1)
            .ok_or(ExtractError::InvariantViolation {
                reason: "stream index overflow".to_string(),
            })?;
    Ok(current)
}

pub(super) fn object_to_f64(value: &Object) -> Result<f64, ExtractError> {
    value
        .as_float()
        .map(f64::from)
        .map_err(|error| ExtractError::ContentDecode {
            reason: error.to_string(),
        })
}
