use crate::{CancellationToken, ExtractError, FontDescriptor, FontId, PageNumber};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

#[derive(Debug, Clone, Copy)]
pub(crate) enum ExtractionStage {
    ExtractStart,
    DocumentLoaded,
    PageIteration,
    ExtractPageStart,
    ExtractPageElementsStart,
    ExtractPageStream,
    ProcessStreamDecode,
    ProcessStreamParseOperation,
    ProcessStreamOperation,
    FormXObjectEnter,
    DecodeContentStreamStart,
    DecodeContentStreamFilter,
    DecodeZlibChunk,
    DecodeDeflateFallbackChunk,
    DecodeLzwChunk,
    DecodeAscii85,
    DecodeAsciiHex,
    DecodeRunLength,
    ContentParseOperation,
    ContentParseToken,
}

impl ExtractionStage {
    const fn as_str(self) -> &'static str {
        match self {
            Self::ExtractStart => "extract_start",
            Self::DocumentLoaded => "document_loaded",
            Self::PageIteration => "page_iteration",
            Self::ExtractPageStart => "extract_page_start",
            Self::ExtractPageElementsStart => "extract_page_elements_start",
            Self::ExtractPageStream => "extract_page_stream",
            Self::ProcessStreamDecode => "process_stream_decode",
            Self::ProcessStreamParseOperation => "process_stream_parse_operation",
            Self::ProcessStreamOperation => "process_stream_operation",
            Self::FormXObjectEnter => "form_xobject_enter",
            Self::DecodeContentStreamStart => "decode_content_stream_start",
            Self::DecodeContentStreamFilter => "decode_content_stream_filter",
            Self::DecodeZlibChunk => "decode_zlib_chunk",
            Self::DecodeDeflateFallbackChunk => "decode_deflate_fallback_chunk",
            Self::DecodeLzwChunk => "decode_lzw_chunk",
            Self::DecodeAscii85 => "decode_ascii85",
            Self::DecodeAsciiHex => "decode_ascii_hex",
            Self::DecodeRunLength => "decode_run_length",
            Self::ContentParseOperation => "content_parse_operation",
            Self::ContentParseToken => "content_parse_token",
        }
    }
}

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

    pub(crate) fn lookup(&self, name: &str) -> Option<FontId> {
        self.ids_by_name.get(name).copied()
    }

    #[must_use]
    pub(crate) fn into_descriptors(self) -> Vec<FontDescriptor> {
        self.descriptors
    }
}

/// Access to the font registry during element emission.
///
/// Serial extraction interns font names on demand (`Mutable`). Parallel
/// extraction runs an explicit pre-pass that populates every font name
/// upfront, then dispatches lock-free with `ReadOnly` access — lookups only,
/// no allocation. The invariant that every lookup hits is guaranteed by
/// `prepopulate_font_registry` walking the same `FontCatalog::iter_display_names`
/// that emission consults.
pub(crate) enum FontRegistryAccess<'a> {
    Mutable(&'a mut FontRegistry),
    ReadOnly(&'a FontRegistry),
}

impl FontRegistryAccess<'_> {
    pub(crate) fn intern_or_lookup(&mut self, name: &str) -> Result<FontId, ExtractError> {
        match self {
            Self::Mutable(registry) => registry.intern(name),
            Self::ReadOnly(registry) => {
                registry
                    .lookup(name)
                    .ok_or_else(|| ExtractError::InvariantViolation {
                        reason: format!(
                            "font `{name}` was not present in pre-populated registry during \
                         parallel extraction"
                        ),
                    })
            }
        }
    }
}

/// Decoded-content-stream budget shared across pages.
///
/// Reservation is a single atomic `fetch_update`: the caller subtracts the
/// number of bytes it is about to emit; on `None` (would underflow) the call
/// fails immediately with `DecodedStreamBudgetExceeded`. This makes the global
/// budget correct under both serial and parallel execution — two racing
/// workers cannot each spend the same stale remainder because the atomic CAS
/// serializes their reservations.
#[derive(Debug)]
pub(crate) struct DecodedBudget {
    remaining: AtomicUsize,
    initial_limit: usize,
}

impl DecodedBudget {
    #[must_use]
    pub(crate) const fn new(initial_limit: usize) -> Self {
        Self {
            remaining: AtomicUsize::new(initial_limit),
            initial_limit,
        }
    }

    pub(crate) fn try_reserve(
        &self,
        requested: usize,
        page_number: PageNumber,
    ) -> Result<(), ExtractError> {
        match self
            .remaining
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current.checked_sub(requested)
            }) {
            Ok(_) => Ok(()),
            Err(remaining_at_fail) => {
                let consumed_before = self.initial_limit.saturating_sub(remaining_at_fail);
                let actual_bytes = consumed_before.saturating_add(requested);
                Err(ExtractError::DecodedStreamBudgetExceeded {
                    page_number: page_number.get(),
                    limit_bytes: self.initial_limit,
                    actual_bytes,
                })
            }
        }
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
        stage: ExtractionStage,
        page_number: Option<PageNumber>,
    ) -> Result<(), ExtractError> {
        if self
            .cancellation_token
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
        {
            return Err(ExtractError::ExtractionCancelled {
                page_number: page_number.map(PageNumber::get),
                stage: stage.as_str(),
            });
        }

        let elapsed_ms = self.started_at.elapsed().as_millis();
        let elapsed_ms = u64::try_from(elapsed_ms).unwrap_or(u64::MAX);
        if elapsed_ms >= self.timeout_ms {
            return Err(ExtractError::ExtractionTimeoutExceeded {
                page_number: page_number.map(PageNumber::get),
                stage: stage.as_str(),
                timeout_ms: self.timeout_ms,
                elapsed_ms,
            });
        }

        Ok(())
    }
}
