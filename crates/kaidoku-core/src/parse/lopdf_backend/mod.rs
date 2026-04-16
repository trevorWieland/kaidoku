pub(crate) mod emit;
pub(crate) mod resources;

use super::ParserBackend;
use crate::{
    BACKEND_ID, ExtractError, ExtractOptions, ExtractionDocument, ExtractionPage, ExtractionSource,
    PageNumber, SCHEMA_VERSION, Sha256Digest,
};
use emit::{ExtractionControl, ExtractionLimits, ExtractionStage, FontRegistry, PageEmitConfig};
use lopdf::{Document, ObjectId};
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

        let mut extracted_pages = Vec::new();
        let mut font_registry = FontRegistry::default();
        let mut remaining_decoded_budget = options.max_total_decoded_stream_bytes();
        for (page_number, page_id) in pages {
            control.checkpoint(
                ExtractionStage::PageIteration,
                PageNumber::new(page_number).ok(),
            )?;
            if !selection.includes(page_number) {
                continue;
            }
            extracted_pages.push(extract_page(
                &document,
                page_number,
                page_id,
                &options,
                &control,
                &mut font_registry,
                &mut remaining_decoded_budget,
            )?);
        }

        if extracted_pages.is_empty() {
            return Err(ExtractError::EmptySelection);
        }

        let source = ExtractionSource {
            backend: BACKEND_ID,
            input_sha256: sha256_digest(input_bytes),
            input_bytes: input_bytes.len(),
        };

        Ok(ExtractionDocument {
            schema_version: SCHEMA_VERSION,
            source,
            fonts: font_registry.into_descriptors(),
            pages: extracted_pages,
        })
    }
}

fn extract_page(
    document: &Document,
    page_number: u32,
    page_id: ObjectId,
    options: &ExtractOptions,
    control: &ExtractionControl,
    font_registry: &mut FontRegistry,
    remaining_decoded_budget: &mut usize,
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
                total_stream_budget: options.max_total_decoded_stream_bytes(),
                max_form_depth: options.max_form_xobject_depth(),
                max_form_visits: options.max_form_xobject_visits(),
            },
            control,
        },
        font_registry,
        remaining_decoded_budget,
    )?;

    Ok(ExtractionPage {
        page_number,
        width: page_geometry.width,
        height: page_geometry.height,
        elements,
    })
}

fn sha256_digest(input_bytes: &[u8]) -> Sha256Digest {
    let mut hasher = Sha256::new();
    hasher.update(input_bytes);
    let digest = hasher.finalize();
    let hex = format!("{digest:x}");
    Sha256Digest::new(hex).expect("sha256 output should always be valid")
}
