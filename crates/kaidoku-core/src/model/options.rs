use super::paging::{PageSelection, ParseBackend};
use std::fmt;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use thiserror::Error;

const DEFAULT_COORDINATE_PRECISION: u8 = 3;
const MAX_COORDINATE_PRECISION: u8 = 6;

pub const fn default_max_input_bytes() -> usize {
    64 * 1024 * 1024
}

pub const fn default_max_pages() -> u32 {
    10_000
}

pub const fn default_max_operations_per_page() -> u32 {
    250_000
}

pub const fn default_max_elements_per_page() -> u32 {
    2_000_000
}

pub const fn default_max_content_stream_bytes() -> usize {
    32 * 1024 * 1024
}

pub const fn default_max_total_decoded_stream_bytes() -> usize {
    128 * 1024 * 1024
}

pub const fn default_max_page_tree_depth() -> usize {
    128
}

pub const fn default_max_form_xobject_depth() -> usize {
    16
}

pub const fn default_max_form_xobject_visits() -> usize {
    2048
}

pub const fn default_max_wall_time_ms() -> u64 {
    30_000
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ExtractOptionsError {
    #[error("option `{name}` must be >= 1")]
    ZeroOrMissingLimit { name: &'static str },
    #[error("coordinate precision {actual} is outside supported range {min}..={max}")]
    InvalidCoordinatePrecision { actual: u8, min: u8, max: u8 },
}

#[derive(Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}

impl fmt::Debug for CancellationToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CancellationToken")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct ExtractOptions {
    backend: ParseBackend,
    page_selection: PageSelection,
    coordinate_precision: u8,
    max_input_bytes: usize,
    max_pages: u32,
    max_operations_per_page: u32,
    max_elements_per_page: u32,
    max_content_stream_bytes: usize,
    max_total_decoded_stream_bytes: usize,
    max_page_tree_depth: usize,
    max_form_xobject_depth: usize,
    max_form_xobject_visits: usize,
    max_wall_time_ms: u64,
    cancellation_token: Option<CancellationToken>,
}

impl ExtractOptions {
    #[must_use]
    pub fn builder() -> ExtractOptionsBuilder {
        ExtractOptionsBuilder::default()
    }

    #[must_use]
    pub fn backend(&self) -> ParseBackend {
        self.backend
    }

    #[must_use]
    pub fn page_selection(&self) -> &PageSelection {
        &self.page_selection
    }

    #[must_use]
    pub fn coordinate_precision(&self) -> u8 {
        self.coordinate_precision
    }

    #[must_use]
    pub fn max_input_bytes(&self) -> usize {
        self.max_input_bytes
    }

    #[must_use]
    pub fn max_pages(&self) -> u32 {
        self.max_pages
    }

    #[must_use]
    pub fn max_operations_per_page(&self) -> u32 {
        self.max_operations_per_page
    }

    #[must_use]
    pub fn max_elements_per_page(&self) -> u32 {
        self.max_elements_per_page
    }

    #[must_use]
    pub fn max_content_stream_bytes(&self) -> usize {
        self.max_content_stream_bytes
    }

    #[must_use]
    pub fn max_total_decoded_stream_bytes(&self) -> usize {
        self.max_total_decoded_stream_bytes
    }

    #[must_use]
    pub fn max_page_tree_depth(&self) -> usize {
        self.max_page_tree_depth
    }

    #[must_use]
    pub fn max_form_xobject_depth(&self) -> usize {
        self.max_form_xobject_depth
    }

    #[must_use]
    pub fn max_form_xobject_visits(&self) -> usize {
        self.max_form_xobject_visits
    }

    #[must_use]
    pub fn max_wall_time_ms(&self) -> u64 {
        self.max_wall_time_ms
    }

    #[must_use]
    pub fn cancellation_token(&self) -> Option<CancellationToken> {
        self.cancellation_token.clone()
    }

    #[must_use]
    pub fn with_page_selection(mut self, page_selection: PageSelection) -> Self {
        self.page_selection = page_selection;
        self
    }

    #[must_use]
    pub fn with_backend(mut self, backend: ParseBackend) -> Self {
        self.backend = backend;
        self
    }
}

impl Default for ExtractOptions {
    fn default() -> Self {
        ExtractOptionsBuilder::default()
            .build()
            .expect("default extract options should always be valid")
    }
}

#[derive(Debug, Clone)]
pub struct ExtractOptionsBuilder {
    backend: ParseBackend,
    page_selection: PageSelection,
    coordinate_precision: u8,
    max_input_bytes: usize,
    max_pages: u32,
    max_operations_per_page: u32,
    max_elements_per_page: u32,
    max_content_stream_bytes: usize,
    max_total_decoded_stream_bytes: usize,
    max_page_tree_depth: usize,
    max_form_xobject_depth: usize,
    max_form_xobject_visits: usize,
    max_wall_time_ms: u64,
    cancellation_token: Option<CancellationToken>,
}

impl Default for ExtractOptionsBuilder {
    fn default() -> Self {
        Self {
            backend: ParseBackend::default(),
            page_selection: PageSelection::All,
            coordinate_precision: DEFAULT_COORDINATE_PRECISION,
            max_input_bytes: default_max_input_bytes(),
            max_pages: default_max_pages(),
            max_operations_per_page: default_max_operations_per_page(),
            max_elements_per_page: default_max_elements_per_page(),
            max_content_stream_bytes: default_max_content_stream_bytes(),
            max_total_decoded_stream_bytes: default_max_total_decoded_stream_bytes(),
            max_page_tree_depth: default_max_page_tree_depth(),
            max_form_xobject_depth: default_max_form_xobject_depth(),
            max_form_xobject_visits: default_max_form_xobject_visits(),
            max_wall_time_ms: default_max_wall_time_ms(),
            cancellation_token: None,
        }
    }
}

impl ExtractOptionsBuilder {
    #[must_use]
    pub fn backend(mut self, backend: ParseBackend) -> Self {
        self.backend = backend;
        self
    }

    #[must_use]
    pub fn page_selection(mut self, page_selection: PageSelection) -> Self {
        self.page_selection = page_selection;
        self
    }

    #[must_use]
    pub fn coordinate_precision(mut self, coordinate_precision: u8) -> Self {
        self.coordinate_precision = coordinate_precision;
        self
    }

    #[must_use]
    pub fn max_input_bytes(mut self, max_input_bytes: usize) -> Self {
        self.max_input_bytes = max_input_bytes;
        self
    }

    #[must_use]
    pub fn max_pages(mut self, max_pages: u32) -> Self {
        self.max_pages = max_pages;
        self
    }

    #[must_use]
    pub fn max_operations_per_page(mut self, max_operations_per_page: u32) -> Self {
        self.max_operations_per_page = max_operations_per_page;
        self
    }

    #[must_use]
    pub fn max_elements_per_page(mut self, max_elements_per_page: u32) -> Self {
        self.max_elements_per_page = max_elements_per_page;
        self
    }

    #[must_use]
    pub fn max_content_stream_bytes(mut self, max_content_stream_bytes: usize) -> Self {
        self.max_content_stream_bytes = max_content_stream_bytes;
        self
    }

    #[must_use]
    pub fn max_total_decoded_stream_bytes(mut self, max_total_decoded_stream_bytes: usize) -> Self {
        self.max_total_decoded_stream_bytes = max_total_decoded_stream_bytes;
        self
    }

    #[must_use]
    pub fn max_page_tree_depth(mut self, max_page_tree_depth: usize) -> Self {
        self.max_page_tree_depth = max_page_tree_depth;
        self
    }

    #[must_use]
    pub fn max_form_xobject_depth(mut self, max_form_xobject_depth: usize) -> Self {
        self.max_form_xobject_depth = max_form_xobject_depth;
        self
    }

    #[must_use]
    pub fn max_form_xobject_visits(mut self, max_form_xobject_visits: usize) -> Self {
        self.max_form_xobject_visits = max_form_xobject_visits;
        self
    }

    #[must_use]
    pub fn max_wall_time_ms(mut self, max_wall_time_ms: u64) -> Self {
        self.max_wall_time_ms = max_wall_time_ms;
        self
    }

    #[must_use]
    pub fn cancellation_token(mut self, cancellation_token: CancellationToken) -> Self {
        self.cancellation_token = Some(cancellation_token);
        self
    }

    pub fn build(self) -> Result<ExtractOptions, ExtractOptionsError> {
        validate_non_zero("max_input_bytes", self.max_input_bytes as u128)?;
        validate_non_zero("max_pages", u128::from(self.max_pages))?;
        validate_non_zero(
            "max_operations_per_page",
            u128::from(self.max_operations_per_page),
        )?;
        validate_non_zero(
            "max_elements_per_page",
            u128::from(self.max_elements_per_page),
        )?;
        validate_non_zero(
            "max_content_stream_bytes",
            self.max_content_stream_bytes as u128,
        )?;
        validate_non_zero(
            "max_total_decoded_stream_bytes",
            self.max_total_decoded_stream_bytes as u128,
        )?;
        validate_non_zero("max_page_tree_depth", self.max_page_tree_depth as u128)?;
        validate_non_zero(
            "max_form_xobject_depth",
            self.max_form_xobject_depth as u128,
        )?;
        validate_non_zero(
            "max_form_xobject_visits",
            self.max_form_xobject_visits as u128,
        )?;
        validate_non_zero("max_wall_time_ms", u128::from(self.max_wall_time_ms))?;

        if self.coordinate_precision > MAX_COORDINATE_PRECISION {
            return Err(ExtractOptionsError::InvalidCoordinatePrecision {
                actual: self.coordinate_precision,
                min: 0,
                max: MAX_COORDINATE_PRECISION,
            });
        }

        Ok(ExtractOptions {
            backend: self.backend,
            page_selection: self.page_selection,
            coordinate_precision: self.coordinate_precision,
            max_input_bytes: self.max_input_bytes,
            max_pages: self.max_pages,
            max_operations_per_page: self.max_operations_per_page,
            max_elements_per_page: self.max_elements_per_page,
            max_content_stream_bytes: self.max_content_stream_bytes,
            max_total_decoded_stream_bytes: self.max_total_decoded_stream_bytes,
            max_page_tree_depth: self.max_page_tree_depth,
            max_form_xobject_depth: self.max_form_xobject_depth,
            max_form_xobject_visits: self.max_form_xobject_visits,
            max_wall_time_ms: self.max_wall_time_ms,
            cancellation_token: self.cancellation_token,
        })
    }
}

fn validate_non_zero(name: &'static str, value: u128) -> Result<(), ExtractOptionsError> {
    if value == 0 {
        return Err(ExtractOptionsError::ZeroOrMissingLimit { name });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{CancellationToken, ExtractOptions, ExtractOptionsError};
    use crate::ParseBackend;

    #[test]
    fn extract_options_defaults_are_stable() {
        let defaults = ExtractOptions::default();
        assert_eq!(defaults.backend(), ParseBackend::Lopdf);
        assert_eq!(defaults.coordinate_precision(), 3);
        assert!(defaults.max_content_stream_bytes() > 0);
        assert!(defaults.max_total_decoded_stream_bytes() >= defaults.max_content_stream_bytes());
        assert!(defaults.max_page_tree_depth() > 0);
        assert!(defaults.max_form_xobject_depth() > 0);
        assert!(defaults.max_form_xobject_visits() > 0);
        assert_eq!(defaults.max_wall_time_ms(), 30_000);
    }

    #[test]
    fn extract_options_builder_rejects_invalid_inputs() {
        let invalid_precision = ExtractOptions::builder().coordinate_precision(7).build();
        assert!(matches!(
            invalid_precision,
            Err(ExtractOptionsError::InvalidCoordinatePrecision {
                actual: 7,
                min: 0,
                max: 6,
            })
        ));

        let zero_limit = ExtractOptions::builder().max_wall_time_ms(0).build();
        assert!(matches!(
            zero_limit,
            Err(ExtractOptionsError::ZeroOrMissingLimit {
                name: "max_wall_time_ms"
            })
        ));
    }

    #[test]
    fn cancellation_token_reports_cancelled_state() {
        let token = CancellationToken::new();
        assert!(!token.is_cancelled());
        token.cancel();
        assert!(token.is_cancelled());
    }
}
