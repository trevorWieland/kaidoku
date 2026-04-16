pub(crate) mod emit;
pub(crate) mod resources;

#[cfg(test)]
mod parallel_extraction_tests;

use super::ParserBackend;
use crate::{
    BACKEND_ID, ExtractError, ExtractOptions, ExtractionDocument, ExtractionPage, ExtractionSource,
    PageNumber, SCHEMA_VERSION, Sha256Digest,
};
use emit::{
    DecodedBudget, ExtractionControl, ExtractionLimits, ExtractionStage, FontRegistry,
    FontRegistryAccess, PageEmitConfig,
};
use lopdf::{Document, Object, ObjectId};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy)]
pub(super) struct LopdfBackend;

impl ParserBackend for LopdfBackend {
    fn extract(
        &self,
        input_bytes: &[u8],
        options: ExtractOptions,
    ) -> Result<ExtractionDocument, ExtractError> {
        let control =
            ExtractionControl::new(options.max_wall_time_ms(), options.cancellation_token());
        control.checkpoint(ExtractionStage::ExtractStart, None)?;

        let document = Document::load_mem(input_bytes).map_err(|error| ExtractError::PdfParse {
            reason: error.to_string(),
        })?;
        control.checkpoint(ExtractionStage::DocumentLoaded, None)?;

        let pages = document.get_pages();
        let total_pages =
            u32::try_from(pages.len()).map_err(|_| ExtractError::InvariantViolation {
                reason: "document page count does not fit into u32".to_string(),
            })?;

        if total_pages > options.max_pages() {
            return Err(ExtractError::PageLimitExceeded {
                limit_pages: options.max_pages(),
                actual_pages: total_pages,
            });
        }

        let selection = options.page_selection().clone().validate(total_pages)?;

        let selected_pages: Vec<(u32, ObjectId)> = pages
            .into_iter()
            .filter(|(page_number, _)| selection.includes(*page_number))
            .collect();

        let (extracted_pages, font_registry) = if options.parallel_page_extraction() {
            extract_pages_parallel(&document, &selected_pages, &options, &control)?
        } else {
            extract_pages_serial(&document, &selected_pages, &options, &control)?
        };

        if extracted_pages.is_empty() {
            return Err(ExtractError::EmptySelection);
        }

        let source =
            ExtractionSource::new(BACKEND_ID, sha256_digest(input_bytes), input_bytes.len());

        Ok(ExtractionDocument::new(
            SCHEMA_VERSION,
            source,
            font_registry.into_descriptors(),
            extracted_pages,
        )?)
    }

    fn extract_first_page(
        &self,
        input_bytes: &[u8],
        options: ExtractOptions,
    ) -> Result<ExtractionDocument, ExtractError> {
        let control =
            ExtractionControl::new(options.max_wall_time_ms(), options.cancellation_token());
        control.checkpoint(ExtractionStage::ExtractStart, None)?;

        let document = Document::load_mem(input_bytes).map_err(|error| ExtractError::PdfParse {
            reason: error.to_string(),
        })?;
        control.checkpoint(ExtractionStage::DocumentLoaded, None)?;

        let first_page_id = find_first_page_id(&document, options.max_page_tree_depth())?;

        let mut font_registry = FontRegistry::default();
        let decoded_budget = DecodedBudget::new(options.max_total_decoded_stream_bytes());
        let page = extract_page(
            &document,
            1,
            first_page_id,
            &options,
            &control,
            FontRegistryAccess::Mutable(&mut font_registry),
            &decoded_budget,
        )?;

        let source =
            ExtractionSource::new(BACKEND_ID, sha256_digest(input_bytes), input_bytes.len());

        Ok(ExtractionDocument::new(
            SCHEMA_VERSION,
            source,
            font_registry.into_descriptors(),
            vec![page],
        )?)
    }
}

fn extract_pages_serial(
    document: &Document,
    selected_pages: &[(u32, ObjectId)],
    options: &ExtractOptions,
    control: &ExtractionControl,
) -> Result<(Vec<ExtractionPage>, FontRegistry), ExtractError> {
    let mut extracted_pages = Vec::with_capacity(selected_pages.len());
    let mut font_registry = FontRegistry::default();
    let decoded_budget = DecodedBudget::new(options.max_total_decoded_stream_bytes());
    for (page_number, page_id) in selected_pages {
        control.checkpoint(
            ExtractionStage::PageIteration,
            PageNumber::new(*page_number).ok(),
        )?;
        extracted_pages.push(extract_page(
            document,
            *page_number,
            *page_id,
            options,
            control,
            FontRegistryAccess::Mutable(&mut font_registry),
            &decoded_budget,
        )?);
    }
    Ok((extracted_pages, font_registry))
}

fn extract_pages_parallel(
    document: &Document,
    selected_pages: &[(u32, ObjectId)],
    options: &ExtractOptions,
    control: &ExtractionControl,
) -> Result<(Vec<ExtractionPage>, FontRegistry), ExtractError> {
    use rayon::prelude::*;

    // Deterministic pre-pass: enumerate every font referenced on each selected
    // page in page-number order so font IDs are assigned stably. After this
    // pass the registry is complete — parallel workers only *look up* IDs,
    // never mutate. This is what makes the parallel path actually parallel:
    // no global mutex is held on the hot path.
    let mut font_registry = FontRegistry::default();
    for (page_number, page_id) in selected_pages {
        control.checkpoint(
            ExtractionStage::PageIteration,
            PageNumber::new(*page_number).ok(),
        )?;
        emit::prepopulate_font_registry(document, *page_id, &mut font_registry)?;
    }

    let decoded_budget = DecodedBudget::new(options.max_total_decoded_stream_bytes());
    let font_registry_ref = &font_registry;

    let page_results: Vec<Result<ExtractionPage, ExtractError>> = selected_pages
        .par_iter()
        .map(|(page_number, page_id)| {
            extract_page(
                document,
                *page_number,
                *page_id,
                options,
                control,
                FontRegistryAccess::ReadOnly(font_registry_ref),
                &decoded_budget,
            )
        })
        .collect();

    let mut extracted_pages = Vec::with_capacity(selected_pages.len());
    for result in page_results {
        extracted_pages.push(result?);
    }

    Ok((extracted_pages, font_registry))
}

fn extract_page(
    document: &Document,
    page_number: u32,
    page_id: ObjectId,
    options: &ExtractOptions,
    control: &ExtractionControl,
    font_registry: FontRegistryAccess<'_>,
    decoded_budget: &DecodedBudget,
) -> Result<ExtractionPage, ExtractError> {
    let page_number = PageNumber::new(page_number)?;
    control.checkpoint(ExtractionStage::ExtractPageStart, Some(page_number))?;
    let page_geometry = resources::page_geometry(
        document,
        page_number,
        page_id,
        options.max_page_tree_depth(),
    )?;
    let page_scope = resources::page_resource_scope(document, page_id)?;

    let elements = emit::extract_page_elements(
        document,
        page_number,
        page_id,
        PageEmitConfig {
            coordinate_precision: options.coordinate_precision(),
            page_geometry,
            root_scope: &page_scope,
            limits: ExtractionLimits {
                operation_budget: options.max_operations_per_page(),
                max_elements: options.max_elements_per_page(),
                stream_byte_limit: options.max_content_stream_bytes(),
                max_form_depth: options.max_form_xobject_depth(),
                max_form_visits: options.max_form_xobject_visits(),
                max_content_nesting_depth: options.max_content_nesting_depth(),
            },
            control,
        },
        font_registry,
        decoded_budget,
    )?;

    ExtractionPage::new(
        page_number,
        page_geometry.width,
        page_geometry.height,
        elements,
    )
    .map_err(ExtractError::from)
}

/// Resolve the first page `ObjectId` by walking only the leftmost path of
/// the `/Pages` tree. Skips gathering the full page map.
fn find_first_page_id(document: &Document, max_depth: usize) -> Result<ObjectId, ExtractError> {
    let pages_root = document
        .catalog()
        .and_then(|catalog| catalog.get_deref(b"Pages", document))
        .map_err(|error| ExtractError::PdfParse {
            reason: format!("failed locating /Pages root: {error}"),
        })?
        .as_reference()
        .or_else(|_| {
            document
                .catalog()
                .and_then(|catalog| catalog.get(b"Pages").and_then(Object::as_reference))
                .map_err(|error| ExtractError::PdfParse {
                    reason: format!("failed dereferencing /Pages root: {error}"),
                })
        })?;

    let mut current = pages_root;
    let mut visited = std::collections::HashSet::new();
    for depth in 0..=max_depth {
        if !visited.insert(current) {
            return Err(ExtractError::PageTreeCycleDetected {
                page_number: 1,
                object_number: current.0,
                object_generation: current.1,
            });
        }
        if depth > max_depth {
            return Err(ExtractError::PageTreeDepthExceeded {
                page_number: 1,
                depth,
                limit: max_depth,
            });
        }

        let dict = document
            .get_dictionary(current)
            .map_err(|error| ExtractError::PdfParse {
                reason: format!("failed reading page-tree node: {error}"),
            })?;

        let type_name = dict
            .get(b"Type")
            .ok()
            .and_then(|value| value.as_name().ok());
        let is_page = type_name == Some(b"Page");
        let is_pages = type_name == Some(b"Pages");

        if is_page {
            return Ok(current);
        }

        if !is_pages {
            // Some producers omit /Type; attempt to follow Kids if present.
            if dict.get(b"Kids").is_err() {
                return Err(ExtractError::MalformedPageGeometry {
                    page_number: 1,
                    page_object_number: current.0,
                    page_object_generation: current.1,
                    reason: "page-tree node missing Kids and Type=Page".to_string(),
                });
            }
        }

        let kids = dict
            .get(b"Kids")
            .and_then(Object::as_array)
            .map_err(|error| ExtractError::PdfParse {
                reason: format!("page-tree Kids is not an array: {error}"),
            })?;

        let first_kid = kids
            .first()
            .and_then(|kid| kid.as_reference().ok())
            .ok_or_else(|| ExtractError::MalformedPageGeometry {
                page_number: 1,
                page_object_number: current.0,
                page_object_generation: current.1,
                reason: "page-tree node has empty Kids array".to_string(),
            })?;

        current = first_kid;
    }

    Err(ExtractError::PageTreeDepthExceeded {
        page_number: 1,
        depth: max_depth.saturating_add(1),
        limit: max_depth,
    })
}

fn sha256_digest(input_bytes: &[u8]) -> Sha256Digest {
    let mut hasher = Sha256::new();
    hasher.update(input_bytes);
    let digest = hasher.finalize();
    let hex = format!("{digest:x}");
    Sha256Digest::new(hex).expect("sha256 output should always be valid")
}
