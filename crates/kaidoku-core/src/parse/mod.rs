pub(crate) mod lopdf_backend;

use crate::{ExtractError, ExtractOptions, ExtractionDocument, ParseBackend};
use lopdf_backend::LopdfBackend;

trait ParserBackend {
    fn extract(
        &self,
        input_bytes: &[u8],
        options: ExtractOptions,
    ) -> Result<ExtractionDocument, ExtractError>;

    fn extract_first_page(
        &self,
        input_bytes: &[u8],
        options: ExtractOptions,
    ) -> Result<ExtractionDocument, ExtractError>;
}

pub(crate) fn extract_with_backend(
    input_bytes: &[u8],
    options: ExtractOptions,
) -> Result<ExtractionDocument, ExtractError> {
    let backend = options.backend();
    BACKEND_REGISTRY
        .iter()
        .find(|entry| entry.kind == backend)
        .ok_or_else(|| ExtractError::InvariantViolation {
            reason: format!("unregistered parse backend: {backend:?}"),
        })
        .and_then(|entry| (entry.extractor)(input_bytes, options))
}

pub(crate) fn extract_first_page_with_backend(
    input_bytes: &[u8],
    options: ExtractOptions,
) -> Result<ExtractionDocument, ExtractError> {
    let backend = options.backend();
    BACKEND_REGISTRY
        .iter()
        .find(|entry| entry.kind == backend)
        .ok_or_else(|| ExtractError::InvariantViolation {
            reason: format!("unregistered parse backend: {backend:?}"),
        })
        .and_then(|entry| (entry.first_page_extractor)(input_bytes, options))
}

#[derive(Clone, Copy)]
struct BackendRegistryEntry {
    kind: ParseBackend,
    extractor: fn(&[u8], ExtractOptions) -> Result<ExtractionDocument, ExtractError>,
    first_page_extractor: fn(&[u8], ExtractOptions) -> Result<ExtractionDocument, ExtractError>,
}

const BACKEND_REGISTRY: [BackendRegistryEntry; 1] = [BackendRegistryEntry {
    kind: ParseBackend::Lopdf,
    extractor: extract_with_lopdf,
    first_page_extractor: extract_first_page_with_lopdf,
}];

fn extract_with_lopdf(
    input_bytes: &[u8],
    options: ExtractOptions,
) -> Result<ExtractionDocument, ExtractError> {
    let backend = LopdfBackend;
    backend.extract(input_bytes, options)
}

fn extract_first_page_with_lopdf(
    input_bytes: &[u8],
    options: ExtractOptions,
) -> Result<ExtractionDocument, ExtractError> {
    let backend = LopdfBackend;
    backend.extract_first_page(input_bytes, options)
}
