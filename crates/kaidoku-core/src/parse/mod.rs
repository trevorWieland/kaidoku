mod lopdf_backend;

use crate::{ExtractError, ExtractOptions, ExtractionDocument};
use lopdf_backend::LopdfBackend;

trait ParseBackend {
    fn extract(
        &self,
        input_bytes: &[u8],
        options: ExtractOptions,
    ) -> Result<ExtractionDocument, ExtractError>;
}

pub(crate) fn extract_with_backend(
    input_bytes: &[u8],
    options: ExtractOptions,
) -> Result<ExtractionDocument, ExtractError> {
    let backend = LopdfBackend;
    backend.extract(input_bytes, options)
}
