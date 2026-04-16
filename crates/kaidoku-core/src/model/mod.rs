mod document;
mod extraction;
mod options;
mod paging;
mod primitives;
mod scalars;

pub use document::{
    CharPayload, ExtractionPage, FontDescriptor, ImagePayload, RawElement, SpanPayload,
};
pub use extraction::{ExtractionDocument, ExtractionSource};
pub use options::{
    CancellationToken, ExtractOptions, ExtractOptionsError, default_max_content_nesting_depth,
    default_max_content_stream_bytes, default_max_elements_per_page,
    default_max_form_xobject_depth, default_max_form_xobject_visits, default_max_input_bytes,
    default_max_operations_per_page, default_max_page_tree_depth, default_max_pages,
    default_max_total_decoded_stream_bytes, default_max_wall_time_ms,
};
pub use paging::{PageRange, PageRangeError, PageSelection, ParseBackend};
pub use primitives::{
    BBox, BackendIdentifier, FontId, PageNumber, SchemaIdentifier, Sha256Digest, SourceRef,
    ValidationError,
};
pub use scalars::{NonNegativeFinite, PositiveFinite};
