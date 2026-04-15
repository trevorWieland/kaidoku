mod model;
mod parse;

pub use model::{
    BBox, CharPayload, ElementKind, ExtractOptions, ExtractionDocument, ExtractionPage,
    ExtractionSource, ImagePayload, PageNumber, PageRange, PageRangeError, PageSelection,
    RawElement, RawPayload, SourceRef, SpanPayload, ValidationError,
    default_max_content_stream_bytes, default_max_elements_per_page, default_max_input_bytes,
    default_max_operations_per_page, default_max_pages,
};

use parse::extract_with_backend;
use serde_json::Error as JsonError;
use thiserror::Error;

pub const SCHEMA_VERSION: &str = "kaidoku.phase1.v1";
pub const BACKEND_ID: &str = "lopdf";

#[derive(Debug, Error)]
pub enum ExtractError {
    #[error("input size {actual_bytes} exceeds limit {limit_bytes}")]
    InputTooLarge {
        limit_bytes: usize,
        actual_bytes: usize,
    },
    #[error("requested page range {start}-{end} is outside document with {total_pages} pages")]
    PageRangeOutOfBounds {
        start: u32,
        end: u32,
        total_pages: u32,
    },
    #[error("requested page {page} is outside document with {total_pages} pages")]
    PageOutOfBounds { page: u32, total_pages: u32 },
    #[error("document page count {actual_pages} exceeds configured limit {limit_pages}")]
    PageLimitExceeded { limit_pages: u32, actual_pages: u32 },
    #[error("page selection cannot be empty")]
    EmptySelection,
    #[error("invalid page range: {0}")]
    InvalidPageRange(#[from] PageRangeError),
    #[error("validation error: {0}")]
    Validation(#[from] ValidationError),
    #[error("pdf parse failed: {reason}")]
    PdfParse { reason: String },
    #[error("content decode failed: {reason}")]
    ContentDecode { reason: String },
    #[error(
        "malformed page geometry on page {page_number} (object {page_object_number}:{page_object_generation}): {reason}"
    )]
    MalformedPageGeometry {
        page_number: u32,
        page_object_number: u32,
        page_object_generation: u16,
        reason: String,
    },
    #[error("extraction limit exceeded on page {page_number}: {kind} {actual} > {limit}")]
    ExtractionLimitExceeded {
        page_number: u32,
        kind: &'static str,
        limit: u64,
        actual: u64,
    },
    #[error("invariant violated: {reason}")]
    InvariantViolation { reason: String },
    #[error("json serialization failed: {0}")]
    JsonSerialization(#[from] JsonError),
}

pub fn extract_pdf(
    input_bytes: &[u8],
    options: ExtractOptions,
) -> Result<ExtractionDocument, ExtractError> {
    if input_bytes.len() > options.max_input_bytes {
        return Err(ExtractError::InputTooLarge {
            limit_bytes: options.max_input_bytes,
            actual_bytes: input_bytes.len(),
        });
    }
    extract_with_backend(input_bytes, options)
}

pub fn to_canonical_json(document: &ExtractionDocument) -> Result<String, ExtractError> {
    let mut json = serde_json::to_string_pretty(document)?;
    json.push('\n');
    Ok(json)
}

#[cfg(test)]
mod phase1_tests;
