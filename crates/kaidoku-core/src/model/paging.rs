use crate::ExtractError;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeSet;
use thiserror::Error;

const DEFAULT_COORDINATE_PRECISION: u8 = 3;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ParseBackend {
    #[default]
    Lopdf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PageRange {
    start: u32,
    end: u32,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PageRangeError {
    #[error("start page must be >= 1")]
    StartPageOutOfRange,
    #[error("end page must be >= start page")]
    EndBeforeStart,
    #[error("explicit page selection cannot be empty")]
    EmptyExplicitSelection,
}

impl<'de> Deserialize<'de> for PageRange {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawPageRange {
            start: u32,
            end: u32,
        }

        let raw = RawPageRange::deserialize(deserializer)?;
        Self::new(raw.start, raw.end).map_err(serde::de::Error::custom)
    }
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
    pub const fn start(self) -> u32 {
        self.start
    }

    #[must_use]
    pub const fn end(self) -> u32 {
        self.end
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExplicitPageSelection {
    pages: Vec<u32>,
}

impl<'de> Deserialize<'de> for ExplicitPageSelection {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let pages = Vec::<u32>::deserialize(deserializer)?;
        Self::from_pages(pages).map_err(serde::de::Error::custom)
    }
}

impl ExplicitPageSelection {
    pub fn from_pages(pages: Vec<u32>) -> Result<Self, PageRangeError> {
        if pages.is_empty() {
            return Err(PageRangeError::EmptyExplicitSelection);
        }

        let mut sorted_pages = BTreeSet::new();
        for page in pages {
            if page == 0 {
                return Err(PageRangeError::StartPageOutOfRange);
            }
            sorted_pages.insert(page);
        }

        Ok(Self {
            pages: sorted_pages.into_iter().collect(),
        })
    }

    #[must_use]
    pub fn includes(&self, page: u32) -> bool {
        self.pages.binary_search(&page).is_ok()
    }

    #[must_use]
    pub fn as_slice(&self) -> &[u32] {
        &self.pages
    }

    pub fn validate_bounds(self, total_pages: u32) -> Result<Self, ExtractError> {
        for page in &self.pages {
            if *page > total_pages {
                return Err(ExtractError::PageOutOfBounds {
                    page: *page,
                    total_pages,
                });
            }
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PageSelection {
    #[default]
    All,
    Range(PageRange),
    Explicit(ExplicitPageSelection),
}

impl PageSelection {
    pub fn from_pages(pages: Vec<u32>) -> Result<Self, PageRangeError> {
        Ok(Self::Explicit(ExplicitPageSelection::from_pages(pages)?))
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
                if range.end() > total_pages {
                    return Err(ExtractError::PageRangeOutOfBounds {
                        start: range.start(),
                        end: range.end(),
                        total_pages,
                    });
                }
                Ok(Self::Range(range))
            }
            Self::Explicit(selection) => {
                Ok(Self::Explicit(selection.validate_bounds(total_pages)?))
            }
        }
    }

    #[must_use]
    pub fn includes(&self, page: u32) -> bool {
        match self {
            Self::All => true,
            Self::Range(range) => range.contains(page),
            Self::Explicit(selection) => selection.includes(page),
        }
    }

    #[must_use]
    pub fn explicit_pages(&self) -> Option<&[u32]> {
        match self {
            Self::Explicit(selection) => Some(selection.as_slice()),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractOptions {
    pub backend: ParseBackend,
    pub page_selection: PageSelection,
    pub coordinate_precision: u8,
    pub max_input_bytes: usize,
    pub max_pages: u32,
    pub max_operations_per_page: u32,
    pub max_elements_per_page: u32,
    pub max_content_stream_bytes: usize,
    pub max_total_decoded_stream_bytes: usize,
    pub max_page_tree_depth: usize,
    pub max_form_xobject_depth: usize,
    pub max_form_xobject_visits: usize,
}

impl Default for ExtractOptions {
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ExplicitPageSelection, ExtractOptions, PageRange, PageRangeError, PageSelection,
        ParseBackend,
    };
    use crate::ExtractError;
    use serde_json::{from_str, to_string};

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
        assert_eq!(range.start(), 2);
        assert_eq!(range.end(), 5);
    }

    #[test]
    fn explicit_page_selection_is_sorted_and_unique() {
        let selection = PageSelection::from_pages(vec![4, 2, 4, 1]);
        assert!(selection.is_ok());

        let Ok(PageSelection::Explicit(selection)) = selection else {
            return;
        };

        assert_eq!(selection.as_slice(), [1, 2, 4]);
    }

    #[test]
    fn explicit_page_selection_rejects_empty() {
        let explicit = ExplicitPageSelection::from_pages(Vec::new());
        assert_eq!(explicit, Err(PageRangeError::EmptyExplicitSelection));
    }

    #[test]
    fn explicit_page_selection_deserialize_is_canonicalized() {
        let parsed = from_str::<ExplicitPageSelection>("[3,1,3,2]");
        assert!(parsed.is_ok());

        let Ok(parsed) = parsed else { return };
        assert_eq!(parsed.as_slice(), [1, 2, 3]);
    }

    #[test]
    fn page_range_deserialize_and_validate_errors() {
        let parsed = from_str::<PageRange>(r#"{"start":2,"end":4}"#);
        assert!(parsed.is_ok());
        let Ok(parsed) = parsed else { return };
        assert_eq!(parsed.start(), 2);
        assert_eq!(parsed.end(), 4);
        assert!(!parsed.is_empty());

        let invalid = from_str::<PageRange>(r#"{"start":0,"end":4}"#);
        assert!(invalid.is_err());
    }

    #[test]
    fn page_selection_validate_catches_out_of_bounds_cases() {
        let empty_doc = PageSelection::All.validate(0);
        assert!(matches!(empty_doc, Err(ExtractError::EmptySelection)));

        let range = PageRange::new(2, 4);
        assert!(range.is_ok());
        let Ok(range) = range else { return };
        let range = PageSelection::Range(range);
        let range_error = range.validate(3);
        assert!(matches!(
            range_error,
            Err(ExtractError::PageRangeOutOfBounds {
                start: 2,
                end: 4,
                total_pages: 3
            })
        ));
    }

    #[test]
    fn explicit_selection_validate_bounds_and_accessors() {
        let explicit = PageSelection::from_pages(vec![2, 5, 3]);
        assert!(explicit.is_ok());
        let Ok(explicit) = explicit else { return };

        assert!(explicit.includes(3));
        assert!(!explicit.includes(1));
        assert_eq!(explicit.explicit_pages(), Some(&[2, 3, 5][..]));

        let out_of_bounds = explicit.clone().validate(4);
        assert!(matches!(
            out_of_bounds,
            Err(ExtractError::PageOutOfBounds {
                page: 5,
                total_pages: 4
            })
        ));

        let in_bounds = explicit.validate(5);
        assert!(in_bounds.is_ok());
    }

    #[test]
    fn parse_backend_and_extract_options_defaults_are_stable() {
        let backend_json = to_string(&ParseBackend::Lopdf);
        assert!(matches!(backend_json, Ok(ref json) if json == "\"lopdf\""));

        let defaults = ExtractOptions::default();
        assert_eq!(defaults.backend, ParseBackend::Lopdf);
        assert_eq!(defaults.coordinate_precision, 3);
        assert!(defaults.max_content_stream_bytes > 0);
        assert!(defaults.max_total_decoded_stream_bytes >= defaults.max_content_stream_bytes);
        assert!(defaults.max_page_tree_depth > 0);
        assert!(defaults.max_form_xobject_depth > 0);
        assert!(defaults.max_form_xobject_visits > 0);
    }
}
