use crate::{CancellationToken, ExtractError, FontDescriptor, FontId, PageNumber};
use std::collections::BTreeMap;
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
