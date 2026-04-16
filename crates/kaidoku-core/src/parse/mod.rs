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

/// Dispatch full extraction to the configured backend via an exhaustive
/// `match` on `ParseBackend`. Adding a new variant is a compile error until a
/// handler is wired here — no runtime registry, no `InvariantViolation`
/// fallback path.
pub(crate) fn extract_with_backend(
    input_bytes: &[u8],
    options: ExtractOptions,
) -> Result<ExtractionDocument, ExtractError> {
    match options.backend() {
        ParseBackend::Lopdf => LopdfBackend.extract(input_bytes, options),
    }
}

/// Dispatch first-page-only extraction to the configured backend via an
/// exhaustive `match`. See [`extract_with_backend`] for rationale.
pub(crate) fn extract_first_page_with_backend(
    input_bytes: &[u8],
    options: ExtractOptions,
) -> Result<ExtractionDocument, ExtractError> {
    match options.backend() {
        ParseBackend::Lopdf => LopdfBackend.extract_first_page(input_bytes, options),
    }
}
