use serde::{Deserialize, Serialize};
use thiserror::Error;

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

    pub const fn len(self) -> u32 {
        self.end - self.start + 1
    }

    pub const fn is_empty(self) -> bool {
        self.len() == 0
    }

    pub const fn contains(self, page: u32) -> bool {
        page >= self.start && page <= self.end
    }
}

#[cfg(test)]
mod tests {
    use super::{PageRange, PageRangeError};

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
}
