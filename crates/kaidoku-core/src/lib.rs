#[cfg(feature = "fuzzing")]
pub mod fuzzing;
mod model;
mod parse;

pub use model::{
    BBox, BackendIdentifier, CancellationToken, CharPayload, ExtractOptions, ExtractOptionsError,
    ExtractionDocument, ExtractionPage, ExtractionSource, FontDescriptor, FontId, ImagePayload,
    NonNegativeFinite, PageNumber, PageRange, PageRangeError, PageSelection, ParseBackend,
    PositiveFinite, RawElement, SchemaIdentifier, Sha256Digest, SourceRef, SpanPayload,
    ValidationError, default_max_content_nesting_depth, default_max_content_stream_bytes,
    default_max_elements_per_page, default_max_form_xobject_depth, default_max_form_xobject_visits,
    default_max_input_bytes, default_max_operations_per_page, default_max_page_tree_depth,
    default_max_pages, default_max_total_decoded_stream_bytes, default_max_wall_time_ms,
};

use parse::{extract_first_page_with_backend, extract_with_backend};
use serde_json::Error as JsonError;
use thiserror::Error;

pub const SCHEMA_VERSION: SchemaIdentifier = SchemaIdentifier::Phase1V2;
pub const BACKEND_ID: BackendIdentifier = BackendIdentifier::Lopdf;

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
        "decoded content stream on page {page_number} stream {stream_index} exceeds byte cap {limit_bytes} with {actual_bytes} bytes"
    )]
    ContentStreamDecodeLimitExceeded {
        page_number: u32,
        stream_index: u32,
        limit_bytes: usize,
        actual_bytes: usize,
    },
    #[error(
        "decoded content-stream budget exceeded on page {page_number}: {actual_bytes} > {limit_bytes}"
    )]
    DecodedStreamBudgetExceeded {
        page_number: u32,
        limit_bytes: usize,
        actual_bytes: usize,
    },
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
    #[error(
        "page-tree cycle detected while resolving geometry on page {page_number} at object {object_number}:{object_generation}"
    )]
    PageTreeCycleDetected {
        page_number: u32,
        object_number: u32,
        object_generation: u16,
    },
    #[error(
        "page-tree traversal depth exceeded while resolving geometry on page {page_number}: {depth} > {limit}"
    )]
    PageTreeDepthExceeded {
        page_number: u32,
        depth: usize,
        limit: usize,
    },
    #[error("form xobject recursion depth exceeded on page {page_number}: {depth} > {limit}")]
    FormXObjectDepthExceeded {
        page_number: u32,
        depth: usize,
        limit: usize,
    },
    #[error("content stream nesting depth exceeded on page {page_number}: {depth} > {limit}")]
    ContentNestingLimitExceeded {
        page_number: u32,
        depth: usize,
        limit: usize,
    },
    #[error(
        "form xobject cycle detected on page {page_number} at object {object_number}:{object_generation}"
    )]
    FormXObjectCycleDetected {
        page_number: u32,
        object_number: u32,
        object_generation: u16,
    },
    #[error("form xobject visit limit exceeded on page {page_number}: {actual} > {limit}")]
    FormXObjectVisitLimitExceeded {
        page_number: u32,
        limit: usize,
        actual: usize,
    },
    #[error(
        "extraction timeout exceeded at stage {stage} after {elapsed_ms}ms (limit {timeout_ms}ms)"
    )]
    ExtractionTimeoutExceeded {
        page_number: Option<u32>,
        stage: &'static str,
        timeout_ms: u64,
        elapsed_ms: u64,
    },
    #[error("extraction cancelled at stage {stage}")]
    ExtractionCancelled {
        page_number: Option<u32>,
        stage: &'static str,
    },
    #[error("invalid fallback geometry for page {page_number}: {reason}")]
    InvalidFallbackGeometry { page_number: u32, reason: String },
    #[error("invariant violated: {reason}")]
    InvariantViolation { reason: String },
    #[error("json serialization failed: {0}")]
    JsonSerialization(#[from] JsonError),
}

pub fn extract_pdf(
    input_bytes: &[u8],
    options: ExtractOptions,
) -> Result<ExtractionDocument, ExtractError> {
    if input_bytes.len() > options.max_input_bytes() {
        return Err(ExtractError::InputTooLarge {
            limit_bytes: options.max_input_bytes(),
            actual_bytes: input_bytes.len(),
        });
    }
    extract_with_backend(input_bytes, options)
}

/// Extract only the first page of the PDF without walking the full page tree.
///
/// This is a low-latency fast path for first-byte/first-page previews. It avoids
/// gathering the complete page map, so scaling is dominated by the single
/// first-page content stream rather than document size.
///
/// The returned [`ExtractionDocument`] contains exactly one page and is
/// byte-identical (after canonical JSON serialization) to
/// `extract_pdf(bytes, opts.page_selection(PageSelection::Range(PageRange::new(1,1)?)))`.
pub fn extract_pdf_first_page(
    input_bytes: &[u8],
    options: ExtractOptions,
) -> Result<ExtractionDocument, ExtractError> {
    if input_bytes.len() > options.max_input_bytes() {
        return Err(ExtractError::InputTooLarge {
            limit_bytes: options.max_input_bytes(),
            actual_bytes: input_bytes.len(),
        });
    }
    extract_first_page_with_backend(input_bytes, options)
}

pub fn to_canonical_json(document: &ExtractionDocument) -> Result<String, ExtractError> {
    let mut json = serde_json::to_string_pretty(document)?;
    json.push('\n');
    Ok(json)
}

#[cfg(test)]
mod phase1_parity_tests;
#[cfg(test)]
mod phase1_support;
#[cfg(test)]
mod phase1_tests;
