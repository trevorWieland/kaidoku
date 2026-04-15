use crate::{CancellationToken, ExtractError, FontDescriptor, FontId, PageNumber};
use std::collections::BTreeMap;
use std::time::Instant;

#[derive(Debug, Clone, Default)]
pub(crate) struct FontRegistry {
    ids_by_name: BTreeMap<String, FontId>,
    descriptors: Vec<FontDescriptor>,
}

impl FontRegistry {
    pub(crate) fn intern(&mut self, name: &str) -> Result<FontId, ExtractError> {
        if let Some(id) = self.ids_by_name.get(name).copied() {
            return Ok(id);
        }

        let next =
            self.descriptors
                .len()
                .checked_add(1)
                .ok_or(ExtractError::InvariantViolation {
                    reason: "font descriptor count overflow".to_string(),
                })?;
        let next = u32::try_from(next).map_err(|_| ExtractError::InvariantViolation {
            reason: "font descriptor id does not fit into u32".to_string(),
        })?;
        let id = FontId::new(next).map_err(ExtractError::from)?;
        self.ids_by_name.insert(name.to_string(), id);
        self.descriptors.push(FontDescriptor {
            id,
            name: name.to_string(),
        });
        Ok(id)
    }

    #[must_use]
    pub(crate) fn into_descriptors(self) -> Vec<FontDescriptor> {
        self.descriptors
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ExtractionControl {
    started_at: Instant,
    timeout_ms: u64,
    cancellation_token: Option<CancellationToken>,
}

impl ExtractionControl {
    #[must_use]
    pub(crate) fn new(timeout_ms: u64, cancellation_token: Option<CancellationToken>) -> Self {
        Self {
            started_at: Instant::now(),
            timeout_ms,
            cancellation_token,
        }
    }

    pub(crate) fn checkpoint(
        &self,
        stage: &'static str,
        page_number: Option<PageNumber>,
    ) -> Result<(), ExtractError> {
        if self
            .cancellation_token
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
        {
            return Err(ExtractError::ExtractionCancelled {
                page_number: page_number.map(PageNumber::get),
                stage,
            });
        }

        let elapsed_ms = self.started_at.elapsed().as_millis();
        let elapsed_ms = u64::try_from(elapsed_ms).unwrap_or(u64::MAX);
        if elapsed_ms >= self.timeout_ms {
            return Err(ExtractError::ExtractionTimeoutExceeded {
                page_number: page_number.map(PageNumber::get),
                stage,
                timeout_ms: self.timeout_ms,
                elapsed_ms,
            });
        }

        Ok(())
    }
}
