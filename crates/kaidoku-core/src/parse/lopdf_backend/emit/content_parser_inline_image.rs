//! Spec-hardened inline image (BI/ID/EI) parsing (ISO 32000-1 §8.9.7).
//!
//! Two code paths are supported:
//!
//! 1. **Length-first**: when the inline image dictionary declares `/L` or
//!    `/Length`, the parser consumes exactly that many bytes of payload and
//!    verifies that the next tokens are `EI` followed by whitespace or a
//!    delimiter. This is unambiguous even when the encoded payload contains
//!    byte sequences that visually resemble `EI`.
//! 2. **EOL-guarded scan**: when no length is declared AND no filter is
//!    applied, we scan forward for the canonical terminator
//!    `EOL EI (whitespace | delimiter | EOF)`. The leading EOL anchor plus
//!    the requirement that the byte following `EI` not be alphanumeric
//!    rejects false matches like `... EID ...` or `... EIR ...` buried in
//!    raw image bytes.
//!
//! When a filter (`/F` or `/Filter`) is declared but no length, we refuse to
//! guess and return a precise `ContentDecode` error naming the filter. This
//! fails fast rather than risking a silently misinterpreted operation stream.

use super::{
    ContentParser, TOKEN_CHECKPOINT_INTERVAL, is_content_space, is_delimiter, is_eol, is_whitespace,
};
use crate::ExtractError;
use lopdf::{Dictionary, Object, Stream, content::Operation};

impl ContentParser<'_> {
    pub(super) fn parse_inline_image(&mut self) -> Result<Operation, ExtractError> {
        self.expect_inline_image_start()?;
        self.skip_content_space()?;

        let mut dict = Dictionary::new();
        loop {
            self.skip_ws_and_comments()?;
            if self.match_keyword(b"ID") {
                // ISO 32000-1 §8.9.7: the `ID` token is followed by exactly
                // one whitespace byte, which is the separator before the
                // image payload begins. Skipping more would consume payload
                // bytes that happen to start with whitespace.
                match self.peek_byte() {
                    Some(byte) if is_content_space(byte) => {
                        self.bump()?;
                    }
                    Some(_) => {
                        return self
                            .content_error("inline image ID token must be followed by whitespace");
                    }
                    None => {
                        return self.content_error("inline image ID terminated before payload");
                    }
                }
                break;
            }

            let key = self.parse_name_bytes()?;
            self.skip_ws_and_comments()?;
            // Inline-image dictionary values are themselves objects; use the
            // depth budget so a pathological nested dictionary here is still
            // caught.
            let value = self.parse_object_at_depth(1)?;
            dict.set(key, value);
        }

        if let Some(declared_length) = declared_length(&dict) {
            return self.consume_inline_image_with_length(dict, declared_length);
        }

        if has_declared_filter(&dict) {
            let filter_name = filter_display_name(&dict).unwrap_or_else(|| "<unknown>".to_string());
            return Err(ExtractError::ContentDecode {
                reason: format!(
                    "inline image declares filter `{filter_name}` without `/Length` or `/L`; \
                     unable to determine payload boundary safely"
                ),
            });
        }

        self.consume_inline_image_with_eol_scan(dict)
    }

    fn expect_inline_image_start(&mut self) -> Result<(), ExtractError> {
        if !self.starts_with(b"BI") || !self.peek_byte_at(2).is_some_and(is_content_space) {
            return self.content_error("inline image operation is missing BI prefix");
        }
        self.cursor = self.cursor.saturating_add(2);
        self.bump_parse_steps()?;
        self.bump_parse_steps()?;
        Ok(())
    }

    fn consume_inline_image_with_length(
        &mut self,
        dict: Dictionary,
        declared_length: usize,
    ) -> Result<Operation, ExtractError> {
        let data_start = self.cursor;
        let data_end =
            data_start
                .checked_add(declared_length)
                .ok_or(ExtractError::InvariantViolation {
                    reason: "inline image length overflow".to_string(),
                })?;
        if data_end > self.bytes.len() {
            return Err(ExtractError::ContentDecode {
                reason: format!(
                    "declared inline image length {declared_length} exceeds remaining stream bytes"
                ),
            });
        }

        let content = self.bytes[data_start..data_end].to_vec();
        self.cursor = data_end;

        // After the payload, PDF requires `EI` optionally preceded by whitespace
        // and followed by whitespace or a delimiter.
        self.skip_content_space()?;
        if !self.match_keyword(b"EI") {
            return self.content_error(
                "inline image payload is not terminated by EI after declared length",
            );
        }
        // match_keyword already verified the following byte is whitespace or
        // delimiter, so any leading-into-next-op whitespace can be eaten here.
        self.skip_content_space()?;
        Ok(Operation {
            operator: "BI".to_string(),
            operands: vec![Object::Stream(Stream::new(dict, content))],
        })
    }

    fn consume_inline_image_with_eol_scan(
        &mut self,
        dict: Dictionary,
    ) -> Result<Operation, ExtractError> {
        let data_start = self.cursor;
        let mut index = data_start;

        while index < self.bytes.len() {
            self.parse_steps = self.parse_steps.saturating_add(1);
            if self.parse_steps % TOKEN_CHECKPOINT_INTERVAL == 0 {
                self.control.checkpoint(
                    super::ExtractionStage::ContentParseToken,
                    Some(self.page_number),
                )?;
            }

            if is_eol(self.bytes[index])
                && let Some(payload_end) = self.match_ei_terminator(index)
            {
                let content = self.bytes[data_start..index].to_vec();
                self.cursor = payload_end;
                self.skip_content_space()?;
                return Ok(Operation {
                    operator: "BI".to_string(),
                    operands: vec![Object::Stream(Stream::new(dict, content))],
                });
            }
            index = index.saturating_add(1);
        }

        self.content_error("inline image data is missing EI terminator")
    }

    /// After the EOL at `eol_index`, is the sequence `(space?)EI(space|delim|EOF)`?
    /// Returns the byte offset immediately after the `EI` token on success.
    fn match_ei_terminator(&self, eol_index: usize) -> Option<usize> {
        let mut cursor = eol_index.checked_add(1)?;
        // Allow one or more whitespace bytes between the EOL anchor and EI.
        while cursor < self.bytes.len() && is_content_space(self.bytes[cursor]) {
            cursor = cursor.checked_add(1)?;
        }
        let e_idx = cursor;
        let i_idx = e_idx.checked_add(1)?;
        let follow_idx = i_idx.checked_add(1)?;
        if i_idx >= self.bytes.len() {
            return None;
        }
        if self.bytes[e_idx] != b'E' || self.bytes[i_idx] != b'I' {
            return None;
        }
        // The byte after EI must be whitespace, a delimiter, or EOF.
        let Some(following) = self.bytes.get(follow_idx).copied() else {
            return Some(follow_idx);
        };
        if is_whitespace(following) || is_delimiter(following) {
            Some(follow_idx)
        } else {
            None
        }
    }
}

fn declared_length(dict: &Dictionary) -> Option<usize> {
    // `/Length` is canonical; some producers emit `/L` as an abbreviation.
    for key in [b"L".as_slice(), b"Length".as_slice()] {
        if let Ok(value) = dict.get(key)
            && let Some(length) = value.as_i64().ok().and_then(|v| usize::try_from(v).ok())
        {
            return Some(length);
        }
    }
    None
}

fn has_declared_filter(dict: &Dictionary) -> bool {
    dict.get(b"F").is_ok() || dict.get(b"Filter").is_ok()
}

fn filter_display_name(dict: &Dictionary) -> Option<String> {
    let object = dict.get(b"F").or_else(|_| dict.get(b"Filter")).ok()?;
    match object {
        Object::Name(name) => Some(String::from_utf8_lossy(name).to_string()),
        Object::Array(items) => items
            .first()
            .and_then(|item| item.as_name().ok())
            .map(|name| String::from_utf8_lossy(name).to_string()),
        _ => None,
    }
}
