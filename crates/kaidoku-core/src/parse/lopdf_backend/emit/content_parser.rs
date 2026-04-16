use super::{ExtractionControl, ExtractionStage};
use crate::{ExtractError, PageNumber};
use lopdf::{Object, content::Operation};

pub(super) const TOKEN_CHECKPOINT_INTERVAL: usize = 256;

#[path = "content_parser_inline_image.rs"]
mod inline_image;
#[path = "content_parser_object_impl.rs"]
mod object_impl;

/// Parse a content stream, invoking `visit` for each operation as it is emitted.
///
/// This is the canonical streaming entrypoint — no intermediate `Vec<Operation>`
/// is built. The callback receives each operation and its monotonic index; if
/// the callback returns `Err`, parsing stops and the error propagates out.
pub(super) fn parse_content_operations_streaming<F>(
    bytes: &[u8],
    page_number: PageNumber,
    max_depth: usize,
    control: &ExtractionControl,
    mut visit: F,
) -> Result<(), ExtractError>
where
    F: FnMut(&Operation, u32) -> Result<(), ExtractError>,
{
    let mut parser = ContentParser {
        bytes,
        cursor: 0,
        page_number,
        max_depth,
        control,
        parse_steps: 0,
    };
    let mut op_index: u32 = 0;
    loop {
        parser
            .control
            .checkpoint(ExtractionStage::ContentParseOperation, Some(page_number))?;
        parser.skip_ws_and_comments()?;
        if parser.eof() {
            break;
        }
        let operation = parser.parse_operation()?;
        visit(&operation, op_index)?;
        op_index = op_index
            .checked_add(1)
            .ok_or(ExtractError::InvariantViolation {
                reason: "content stream operation index overflow".to_string(),
            })?;
    }
    Ok(())
}

/// Convenience helper: collect all operations into a `Vec`. Intended for tests
/// and fuzz harnesses. Production code MUST prefer
/// [`parse_content_operations_streaming`] so the extraction pipeline does not
/// materialize an intermediate buffer.
#[cfg(any(test, feature = "fuzzing"))]
pub(crate) fn parse_content_operations_bounded(
    bytes: &[u8],
    page_number: PageNumber,
    max_depth: usize,
    control: &ExtractionControl,
) -> Result<Vec<Operation>, ExtractError> {
    let mut operations = Vec::new();
    parse_content_operations_streaming(bytes, page_number, max_depth, control, |op, _idx| {
        operations.push(op.clone());
        Ok(())
    })?;
    Ok(operations)
}

pub(super) struct ContentParser<'a> {
    pub(super) bytes: &'a [u8],
    pub(super) cursor: usize,
    pub(super) page_number: PageNumber,
    pub(super) max_depth: usize,
    pub(super) control: &'a ExtractionControl,
    pub(super) parse_steps: usize,
}

impl ContentParser<'_> {
    fn parse_operation(&mut self) -> Result<Operation, ExtractError> {
        if self.starts_with_inline_image() {
            return self.parse_inline_image();
        }

        let mut operands = Vec::new();
        loop {
            self.skip_ws_and_comments()?;
            if self.eof() {
                return self.content_error("unexpected end of stream while parsing operation");
            }

            if self.starts_with_inline_image() && operands.is_empty() {
                return self.parse_inline_image();
            }

            let checkpoint = self.cursor;
            let object_result = self.parse_object();
            match object_result {
                Ok(object) => {
                    operands.push(object);
                    self.skip_ws_and_comments()?;
                    if self.eof() {
                        return self.content_error(
                            "unexpected end of stream while parsing operator token",
                        );
                    }
                    if is_operator_start(self.peek_byte().unwrap_or_default()) {
                        let operator = self.parse_operator()?;
                        return Ok(Operation { operator, operands });
                    }
                }
                Err(err) => {
                    // Structural errors — depth overflow, cooperative
                    // control, invariant violations — must be propagated
                    // directly. Otherwise an adversarial stream can hide a
                    // depth overflow behind a spurious operator-parse error.
                    if matches!(
                        err,
                        ExtractError::ContentNestingLimitExceeded { .. }
                            | ExtractError::ExtractionTimeoutExceeded { .. }
                            | ExtractError::ExtractionCancelled { .. }
                            | ExtractError::InvariantViolation { .. }
                    ) {
                        return Err(err);
                    }
                    self.cursor = checkpoint;
                    let operator = self.parse_operator()?;
                    return Ok(Operation { operator, operands });
                }
            }
        }
    }

    fn parse_operator(&mut self) -> Result<String, ExtractError> {
        let start = self.cursor;
        while let Some(byte) = self.peek_byte() {
            if is_operator_start(byte) {
                self.bump()?;
            } else {
                break;
            }
        }

        if self.cursor == start {
            return self.content_error("failed parsing operator token");
        }

        let operator = String::from_utf8_lossy(&self.bytes[start..self.cursor]).to_string();
        self.skip_content_space()?;
        Ok(operator)
    }

    pub(super) fn parse_object(&mut self) -> Result<Object, ExtractError> {
        self.parse_object_at_depth(0)
    }

    pub(super) fn parse_object_at_depth(&mut self, depth: usize) -> Result<Object, ExtractError> {
        if depth > self.max_depth {
            return Err(ExtractError::ContentNestingLimitExceeded {
                page_number: self.page_number.get(),
                depth,
                limit: self.max_depth,
            });
        }
        self.skip_ws_and_comments()?;
        let Some(byte) = self.peek_byte() else {
            return self.content_error("unexpected end of stream while parsing object");
        };

        match byte {
            b'/' => Ok(Object::Name(self.parse_name_bytes()?)),
            b'(' => self.parse_literal_string(),
            b'[' => self.parse_array(depth),
            b'<' => {
                if self.peek_byte_at(1) == Some(b'<') {
                    self.parse_dictionary(depth)
                } else {
                    self.parse_hex_string()
                }
            }
            b'n' => {
                self.expect_keyword(b"null")?;
                Ok(Object::Null)
            }
            b't' => {
                self.expect_keyword(b"true")?;
                Ok(Object::Boolean(true))
            }
            b'f' => {
                self.expect_keyword(b"false")?;
                Ok(Object::Boolean(false))
            }
            b'+' | b'-' | b'.' | b'0'..=b'9' => self.parse_number(),
            _ => self.content_error("unsupported object token in content stream"),
        }
    }

    pub(super) fn skip_ws_and_comments(&mut self) -> Result<(), ExtractError> {
        loop {
            while let Some(byte) = self.peek_byte() {
                if !is_whitespace(byte) {
                    break;
                }
                self.bump()?;
            }
            if self.peek_byte() != Some(b'%') {
                break;
            }
            while let Some(byte) = self.bump_if_available()? {
                if byte == b'\n' || byte == b'\r' {
                    break;
                }
            }
        }
        Ok(())
    }

    pub(super) fn skip_content_space(&mut self) -> Result<(), ExtractError> {
        while let Some(byte) = self.peek_byte() {
            if !is_content_space(byte) {
                break;
            }
            self.bump()?;
        }
        Ok(())
    }

    fn starts_with_inline_image(&self) -> bool {
        self.starts_with(b"BI") && self.peek_byte_at(2).is_some_and(is_content_space)
    }

    pub(super) fn match_keyword(&mut self, keyword: &[u8]) -> bool {
        if !self.starts_with(keyword) {
            return false;
        }
        let end = self.cursor.saturating_add(keyword.len());
        if let Some(next) = self.bytes.get(end).copied()
            && !is_whitespace(next)
            && !is_delimiter(next)
        {
            return false;
        }
        self.cursor = end;
        true
    }

    pub(super) fn expect_keyword(&mut self, keyword: &[u8]) -> Result<(), ExtractError> {
        if self.match_keyword(keyword) {
            Ok(())
        } else {
            self.content_error("unexpected keyword token in content stream")
        }
    }

    pub(super) fn expect_byte(&mut self, expected: u8) -> Result<(), ExtractError> {
        if self.peek_byte() == Some(expected) {
            self.cursor = self.cursor.saturating_add(1);
            self.bump_parse_steps()?;
            Ok(())
        } else {
            self.content_error("unexpected token in content stream")
        }
    }

    pub(super) fn bump(&mut self) -> Result<u8, ExtractError> {
        let Some(byte) = self.peek_byte() else {
            return self.content_error("unexpected end of content stream");
        };
        self.cursor = self.cursor.saturating_add(1);
        self.bump_parse_steps()?;
        Ok(byte)
    }

    pub(super) fn bump_if_available(&mut self) -> Result<Option<u8>, ExtractError> {
        if self.eof() {
            return Ok(None);
        }
        Ok(Some(self.bump()?))
    }

    pub(super) fn bump_parse_steps(&mut self) -> Result<(), ExtractError> {
        self.parse_steps = self.parse_steps.saturating_add(1);
        if self.parse_steps % TOKEN_CHECKPOINT_INTERVAL == 0 {
            self.control
                .checkpoint(ExtractionStage::ContentParseToken, Some(self.page_number))?;
        }
        Ok(())
    }

    pub(super) fn starts_with(&self, expected: &[u8]) -> bool {
        self.bytes
            .get(self.cursor..self.cursor.saturating_add(expected.len()))
            == Some(expected)
    }

    pub(super) fn peek_byte(&self) -> Option<u8> {
        self.bytes.get(self.cursor).copied()
    }

    pub(super) fn peek_byte_at(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.cursor.saturating_add(offset)).copied()
    }

    pub(super) fn eof(&self) -> bool {
        self.cursor >= self.bytes.len()
    }

    pub(super) fn content_error<T>(&self, message: &str) -> Result<T, ExtractError> {
        Err(ExtractError::ContentDecode {
            reason: format!("{message} at byte offset {}", self.cursor),
        })
    }
}

pub(super) fn is_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0c | 0x00)
}

pub(super) fn is_content_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r')
}

pub(super) fn is_eol(byte: u8) -> bool {
    byte == b'\n' || byte == b'\r'
}

pub(super) fn is_delimiter(byte: u8) -> bool {
    matches!(
        byte,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

fn is_operator_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || matches!(byte, b'*' | b'\'' | b'"')
}

pub(super) fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
#[path = "content_parser_tests.rs"]
mod tests;
