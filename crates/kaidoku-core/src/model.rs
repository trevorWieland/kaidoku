use crate::ExtractError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::num::NonZeroU32;
use thiserror::Error;

const DEFAULT_COORDINATE_PRECISION: u8 = 3;

pub const fn default_max_input_bytes() -> usize {
    64 * 1024 * 1024
}

pub const fn default_max_pages() -> u32 {
    10_000
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageRange {
    pub start: u32,
    pub end: u32,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PageRangeError {
    #[error("start page must be >= 1")]
    StartPageOutOfRange,
    #[error("end page must be >= start page")]
    EndBeforeStart,
}

impl PageRange {
    pub fn new(start: u32, end: u32) -> Result<Self, PageRangeError> {
        if start == 0 {
            return Err(PageRangeError::StartPageOutOfRange);
        }
        if end < start {
            return Err(PageRangeError::EndBeforeStart);
        }
        Ok(Self { start, end })
    }

    #[must_use]
    pub const fn len(self) -> u32 {
        self.end - self.start + 1
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.len() == 0
    }

    #[must_use]
    pub const fn contains(self, page: u32) -> bool {
        page >= self.start && page <= self.end
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PageSelection {
    #[default]
    All,
    Range(PageRange),
    Explicit(Vec<u32>),
}

impl PageSelection {
    pub fn from_pages(pages: Vec<u32>) -> Result<Self, PageRangeError> {
        let mut sorted_pages = BTreeSet::new();
        for page in pages {
            if page == 0 {
                return Err(PageRangeError::StartPageOutOfRange);
            }
            sorted_pages.insert(page);
        }
        Ok(Self::Explicit(sorted_pages.into_iter().collect()))
    }

    pub fn validate(self, total_pages: u32) -> Result<Self, ExtractError> {
        match self {
            Self::All => {
                if total_pages == 0 {
                    return Err(ExtractError::EmptySelection);
                }
                Ok(Self::All)
            }
            Self::Range(range) => {
                if range.end > total_pages {
                    return Err(ExtractError::PageRangeOutOfBounds {
                        start: range.start,
                        end: range.end,
                        total_pages,
                    });
                }
                Ok(Self::Range(range))
            }
            Self::Explicit(pages) => {
                if pages.is_empty() {
                    return Err(ExtractError::EmptySelection);
                }
                for page in &pages {
                    if *page > total_pages {
                        return Err(ExtractError::PageOutOfBounds {
                            page: *page,
                            total_pages,
                        });
                    }
                }
                Ok(Self::Explicit(pages))
            }
        }
    }

    #[must_use]
    pub fn includes(&self, page: u32) -> bool {
        match self {
            Self::All => true,
            Self::Range(range) => range.contains(page),
            Self::Explicit(pages) => pages.binary_search(&page).is_ok(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractOptions {
    pub page_selection: PageSelection,
    pub coordinate_precision: u8,
    pub max_input_bytes: usize,
    pub max_pages: u32,
}

impl Default for ExtractOptions {
    fn default() -> Self {
        Self {
            page_selection: PageSelection::All,
            coordinate_precision: DEFAULT_COORDINATE_PRECISION,
            max_input_bytes: default_max_input_bytes(),
            max_pages: default_max_pages(),
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum ValidationError {
    #[error("bbox coordinates must be finite")]
    NonFiniteCoordinate,
    #[error("bbox width and height must be >= 0")]
    NegativeDimension,
    #[error("page number must be >= 1")]
    InvalidPageNumber,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PageNumber(NonZeroU32);

impl PageNumber {
    pub fn new(value: u32) -> Result<Self, ValidationError> {
        let Some(number) = NonZeroU32::new(value) else {
            return Err(ValidationError::InvalidPageNumber);
        };
        Ok(Self(number))
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0.get()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl BBox {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Result<Self, ValidationError> {
        if !x.is_finite() || !y.is_finite() || !width.is_finite() || !height.is_finite() {
            return Err(ValidationError::NonFiniteCoordinate);
        }
        if width < 0.0 || height < 0.0 {
            return Err(ValidationError::NegativeDimension);
        }
        Ok(Self {
            x,
            y,
            width,
            height,
        })
    }

    #[must_use]
    pub fn quantized(self, precision: u8) -> Self {
        Self {
            x: quantize(self.x, precision),
            y: quantize(self.y, precision),
            width: quantize(self.width, precision),
            height: quantize(self.height, precision),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ElementKind {
    Char,
    Span,
    Image,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CharPayload {
    pub text: String,
    pub font_name: Option<String>,
    pub font_size: f64,
    pub char_index: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpanPayload {
    pub text: String,
    pub font_name: Option<String>,
    pub font_size: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImagePayload {
    pub name: String,
    pub width_px: u32,
    pub height_px: u32,
    pub color_space: Option<String>,
    pub bits_per_component: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "payload_type", rename_all = "snake_case")]
pub enum RawPayload {
    Char(CharPayload),
    Span(SpanPayload),
    Image(ImagePayload),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SourceRef {
    pub page_number: PageNumber,
    pub page_object_number: u32,
    pub page_object_generation: u16,
    pub stream_index: u32,
    pub operation_index: u32,
    pub element_index: u32,
}

impl SourceRef {
    pub fn new(
        page_number: u32,
        page_object_number: u32,
        page_object_generation: u16,
        stream_index: u32,
        operation_index: u32,
        element_index: u32,
    ) -> Result<Self, ValidationError> {
        Ok(Self {
            page_number: PageNumber::new(page_number)?,
            page_object_number,
            page_object_generation,
            stream_index,
            operation_index,
            element_index,
        })
    }

    #[must_use]
    pub fn stable_key(self) -> String {
        format!(
            "p{}-o{}-s{}-i{}",
            self.page_number.get(),
            self.operation_index,
            self.stream_index,
            self.element_index
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawElement {
    pub kind: ElementKind,
    pub bbox: BBox,
    pub payload: RawPayload,
    pub source_ref: SourceRef,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtractionPage {
    pub page_number: PageNumber,
    pub width: f64,
    pub height: f64,
    pub elements: Vec<RawElement>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractionSource {
    pub backend: String,
    pub input_sha256: String,
    pub input_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtractionDocument {
    pub schema_version: String,
    pub source: ExtractionSource,
    pub pages: Vec<ExtractionPage>,
}

fn quantize(value: f64, precision: u8) -> f64 {
    let scale = 10_f64.powi(i32::from(precision));
    (value * scale).round() / scale
}

#[cfg(test)]
mod tests {
    use super::{BBox, PageRange, PageRangeError, PageSelection, SourceRef, ValidationError};

    #[test]
    fn new_rejects_zero_start() {
        let range = PageRange::new(0, 1);
        assert_eq!(range, Err(PageRangeError::StartPageOutOfRange));
    }

    #[test]
    fn new_rejects_end_before_start() {
        let range = PageRange::new(3, 2);
        assert_eq!(range, Err(PageRangeError::EndBeforeStart));
    }

    #[test]
    fn len_and_contains_work_for_valid_range() {
        let range_result = PageRange::new(2, 5);
        assert!(range_result.is_ok());

        let Ok(range) = range_result else { return };

        assert_eq!(range.len(), 4);
        assert!(range.contains(2));
        assert!(range.contains(5));
        assert!(!range.contains(1));
        assert!(!range.contains(6));
    }

    #[test]
    fn bbox_rejects_invalid_values() {
        let invalid_width = BBox::new(0.0, 0.0, -1.0, 1.0);
        assert_eq!(invalid_width, Err(ValidationError::NegativeDimension));

        let invalid_nan = BBox::new(f64::NAN, 0.0, 1.0, 1.0);
        assert_eq!(invalid_nan, Err(ValidationError::NonFiniteCoordinate));
    }

    #[test]
    fn source_ref_stable_key_is_deterministic() {
        let source_ref = SourceRef::new(1, 12, 0, 0, 9, 3);
        assert!(source_ref.is_ok());

        let Ok(source_ref) = source_ref else { return };
        assert_eq!(source_ref.stable_key(), "p1-o9-s0-i3");
    }

    #[test]
    fn explicit_page_selection_is_sorted_and_unique() {
        let selection = PageSelection::from_pages(vec![4, 2, 4, 1]);
        assert!(selection.is_ok());

        let Ok(PageSelection::Explicit(pages)) = selection else {
            return;
        };

        assert_eq!(pages, vec![1, 2, 4]);
    }
}
