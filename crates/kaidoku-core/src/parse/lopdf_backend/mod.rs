mod emit;
mod resources;

use super::ParseBackend;
use crate::{
    BACKEND_ID, ExtractError, ExtractOptions, ExtractionDocument, ExtractionPage, ExtractionSource,
    PageNumber, SCHEMA_VERSION,
};
use lopdf::{Document, ObjectId};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy)]
pub(super) struct LopdfBackend;

impl ParseBackend for LopdfBackend {
    fn extract(
        &self,
        input_bytes: &[u8],
        options: ExtractOptions,
    ) -> Result<ExtractionDocument, ExtractError> {
        let document = Document::load_mem(input_bytes).map_err(|error| ExtractError::PdfParse {
            reason: error.to_string(),
        })?;

        let pages = document.get_pages();
        let total_pages =
            u32::try_from(pages.len()).map_err(|_| ExtractError::InvariantViolation {
                reason: "document page count does not fit into u32".to_string(),
            })?;

        if total_pages > options.max_pages {
            return Err(ExtractError::PageLimitExceeded {
                limit_pages: options.max_pages,
                actual_pages: total_pages,
            });
        }

        let selection = options.page_selection.validate(total_pages)?;

        let mut extracted_pages = Vec::new();
        for (page_number, page_id) in pages {
            if !selection.includes(page_number) {
                continue;
            }
            extracted_pages.push(extract_page(
                &document,
                page_number,
                page_id,
                options.coordinate_precision,
            )?);
        }

        if extracted_pages.is_empty() {
            return Err(ExtractError::EmptySelection);
        }

        let source = ExtractionSource {
            backend: BACKEND_ID.to_string(),
            input_sha256: sha256_hex(input_bytes),
            input_bytes: input_bytes.len(),
        };

        Ok(ExtractionDocument {
            schema_version: SCHEMA_VERSION.to_string(),
            source,
            pages: extracted_pages,
        })
    }
}

fn extract_page(
    document: &Document,
    page_number: u32,
    page_id: ObjectId,
    coordinate_precision: u8,
) -> Result<ExtractionPage, ExtractError> {
    let page_number = PageNumber::new(page_number)?;
    let (page_width, page_height) = resources::page_dimensions(document, page_id);
    let image_catalog = resources::build_image_catalog(document, page_id)?;

    let mut elements = emit::extract_page_elements(
        document,
        page_number,
        page_id,
        coordinate_precision,
        &image_catalog,
    )?;

    elements.sort_by_key(emit::element_sort_key);

    Ok(ExtractionPage {
        page_number,
        width: page_width,
        height: page_height,
        elements,
    })
}

fn sha256_hex(input_bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input_bytes);
    let digest = hasher.finalize();
    format!("{digest:x}")
}
