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
    pub page_selection: PageSelection,
    pub coordinate_precision: u8,
    pub max_input_bytes: usize,
    pub max_pages: u32,
    pub max_operations_per_page: u32,
    pub max_elements_per_page: u32,
    pub max_content_stream_bytes: usize,
}

impl Default for ExtractOptions {
    fn default() -> Self {
        Self {
            page_selection: PageSelection::All,
            coordinate_precision: DEFAULT_COORDINATE_PRECISION,
            max_input_bytes: default_max_input_bytes(),
            max_pages: default_max_pages(),
            max_operations_per_page: default_max_operations_per_page(),
            max_elements_per_page: default_max_elements_per_page(),
            max_content_stream_bytes: default_max_content_stream_bytes(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ExplicitPageSelection, PageRange, PageRangeError, PageSelection};
    use serde_json::from_str;

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
}
