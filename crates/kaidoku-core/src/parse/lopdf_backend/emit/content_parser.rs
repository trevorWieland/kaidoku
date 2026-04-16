use super::{ExtractionControl, ExtractionStage};
use crate::{ExtractError, PageNumber};
use lopdf::{Dictionary, Object, Stream, content::Operation};

const TOKEN_CHECKPOINT_INTERVAL: usize = 256;

#[path = "content_parser_object_impl.rs"]
mod object_impl;

pub(super) fn parse_content_operations_bounded(
    bytes: &[u8],
    page_number: PageNumber,
    control: &ExtractionControl,
) -> Result<Vec<Operation>, ExtractError> {
    let mut parser = ContentParser {
        bytes,
        cursor: 0,
        page_number,
        control,
        parse_steps: 0,
    };
    parser.parse_all()
}

struct ContentParser<'a> {
    bytes: &'a [u8],
    cursor: usize,
    page_number: PageNumber,
    control: &'a ExtractionControl,
    parse_steps: usize,
}

impl ContentParser<'_> {
    fn parse_all(&mut self) -> Result<Vec<Operation>, ExtractError> {
        let mut operations = Vec::new();
        loop {
            self.control.checkpoint(
                ExtractionStage::ContentParseOperation,
                Some(self.page_number),
            )?;
            self.skip_ws_and_comments()?;
            if self.eof() {
                break;
            }
            operations.push(self.parse_operation()?);
        }
        Ok(operations)
    }

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
            if let Ok(object) = self.parse_object() {
                operands.push(object);
                self.skip_ws_and_comments()?;
                if self.eof() {
                    return self
                        .content_error("unexpected end of stream while parsing operator token");
                }
                if is_operator_start(self.peek_byte().unwrap_or_default()) {
                    let operator = self.parse_operator()?;
                    return Ok(Operation { operator, operands });
                }
            } else {
                self.cursor = checkpoint;
                let operator = self.parse_operator()?;
                return Ok(Operation { operator, operands });
            }
        }
    }

    fn parse_inline_image(&mut self) -> Result<Operation, ExtractError> {
        self.expect_inline_image_start()?;
        self.skip_content_space()?;

        let mut dict = Dictionary::new();
        loop {
            self.skip_ws_and_comments()?;
            if self.match_keyword(b"ID") {
                if let Some(byte) = self.peek_byte()
                    && !is_content_space(byte)
                {
                    return self
                        .content_error("inline image ID token must be followed by whitespace");
                }
                self.skip_content_space()?;
                break;
            }

            let key = self.parse_name_bytes()?;
            self.skip_ws_and_comments()?;
            let value = self.parse_object()?;
            dict.set(key, value);
        }

        let data_start = self.cursor;
        let mut index = data_start;
        while index.saturating_add(3) < self.bytes.len() {
            self.parse_steps = self.parse_steps.saturating_add(1);
            if self.parse_steps % TOKEN_CHECKPOINT_INTERVAL == 0 {
                self.control
                    .checkpoint(ExtractionStage::ContentParseToken, Some(self.page_number))?;
            }
            let start = self.bytes[index];
            let e = self.bytes[index + 1];
            let i = self.bytes[index + 2];
            let end = self.bytes[index + 3];
            if is_content_space(start) && e == b'E' && i == b'I' && is_content_space(end) {
                let content = self.bytes[data_start..index].to_vec();
                self.cursor = index + 3;
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

    fn parse_object(&mut self) -> Result<Object, ExtractError> {
        self.skip_ws_and_comments()?;
        let Some(byte) = self.peek_byte() else {
            return self.content_error("unexpected end of stream while parsing object");
        };

        match byte {
            b'/' => Ok(Object::Name(self.parse_name_bytes()?)),
            b'(' => self.parse_literal_string(),
            b'[' => self.parse_array(),
            b'<' => {
                if self.peek_byte_at(1) == Some(b'<') {
                    self.parse_dictionary()
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

    fn skip_ws_and_comments(&mut self) -> Result<(), ExtractError> {
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

    fn skip_content_space(&mut self) -> Result<(), ExtractError> {
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

    fn expect_inline_image_start(&mut self) -> Result<(), ExtractError> {
        if !self.starts_with_inline_image() {
            return self.content_error("inline image operation is missing BI prefix");
        }
        self.cursor = self.cursor.saturating_add(2);
        self.bump_parse_steps()?;
        self.bump_parse_steps()?;
        Ok(())
    }

    fn match_keyword(&mut self, keyword: &[u8]) -> bool {
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

    fn expect_keyword(&mut self, keyword: &[u8]) -> Result<(), ExtractError> {
        if self.match_keyword(keyword) {
            Ok(())
        } else {
            self.content_error("unexpected keyword token in content stream")
        }
    }

    fn expect_byte(&mut self, expected: u8) -> Result<(), ExtractError> {
        if self.peek_byte() == Some(expected) {
            self.cursor = self.cursor.saturating_add(1);
            self.bump_parse_steps()?;
            Ok(())
        } else {
            self.content_error("unexpected token in content stream")
        }
    }

    fn bump(&mut self) -> Result<u8, ExtractError> {
        let Some(byte) = self.peek_byte() else {
            return self.content_error("unexpected end of content stream");
        };
        self.cursor = self.cursor.saturating_add(1);
        self.bump_parse_steps()?;
        Ok(byte)
    }

    fn bump_if_available(&mut self) -> Result<Option<u8>, ExtractError> {
        if self.eof() {
            return Ok(None);
        }
        Ok(Some(self.bump()?))
    }

    fn bump_parse_steps(&mut self) -> Result<(), ExtractError> {
        self.parse_steps = self.parse_steps.saturating_add(1);
        if self.parse_steps % TOKEN_CHECKPOINT_INTERVAL == 0 {
            self.control
                .checkpoint(ExtractionStage::ContentParseToken, Some(self.page_number))?;
        }
        Ok(())
    }

    fn starts_with(&self, expected: &[u8]) -> bool {
        self.bytes
            .get(self.cursor..self.cursor.saturating_add(expected.len()))
            == Some(expected)
    }

    fn peek_byte(&self) -> Option<u8> {
        self.bytes.get(self.cursor).copied()
    }

    fn peek_byte_at(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.cursor.saturating_add(offset)).copied()
    }

    fn eof(&self) -> bool {
        self.cursor >= self.bytes.len()
    }

    fn content_error<T>(&self, message: &str) -> Result<T, ExtractError> {
        Err(ExtractError::ContentDecode {
            reason: format!("{message} at byte offset {}", self.cursor),
        })
    }
}

fn is_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0c | 0x00)
}

fn is_content_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r')
}

fn is_delimiter(byte: u8) -> bool {
    matches!(
        byte,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

fn is_operator_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || matches!(byte, b'*' | b'\'' | b'"')
}

fn hex_nibble(byte: u8) -> Option<u8> {
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
