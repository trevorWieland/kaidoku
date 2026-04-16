use super::{ContentParser, hex_nibble, is_delimiter, is_whitespace};
use crate::ExtractError;
use lopdf::{Dictionary, Object, StringFormat};

impl ContentParser<'_> {
    pub(super) fn parse_name_bytes(&mut self) -> Result<Vec<u8>, ExtractError> {
        self.expect_byte(b'/')?;
        let mut name = Vec::new();
        while let Some(byte) = self.peek_byte() {
            if is_whitespace(byte) || is_delimiter(byte) {
                break;
            }
            if byte == b'#' {
                self.bump()?;
                let high = self.bump()?;
                let low = self.bump()?;
                let Some(high) = hex_nibble(high) else {
                    return self.content_error("invalid hex escape in name");
                };
                let Some(low) = hex_nibble(low) else {
                    return self.content_error("invalid hex escape in name");
                };
                name.push((high << 4) | low);
                continue;
            }
            name.push(self.bump()?);
        }
        Ok(name)
    }

    pub(super) fn parse_literal_string(&mut self) -> Result<Object, ExtractError> {
        self.expect_byte(b'(')?;
        let mut content = Vec::new();
        let mut depth: usize = 1;

        while let Some(byte) = self.bump_if_available()? {
            match byte {
                b'(' => {
                    depth = depth.saturating_add(1);
                    content.push(byte);
                }
                b')' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return Ok(Object::String(content, StringFormat::Literal));
                    }
                    content.push(byte);
                }
                b'\\' => {
                    let Some(escaped) = self.bump_if_available()? else {
                        return self
                            .content_error("unterminated escape sequence in literal string");
                    };
                    match escaped {
                        b'n' => content.push(b'\n'),
                        b'r' => content.push(b'\r'),
                        b't' => content.push(b'\t'),
                        b'b' => content.push(0x08),
                        b'f' => content.push(0x0c),
                        b'\n' => {}
                        b'\r' => {
                            if self.peek_byte() == Some(b'\n') {
                                self.bump()?;
                            }
                        }
                        b'0'..=b'7' => {
                            let mut octal = vec![escaped];
                            for _ in 0..2 {
                                let Some(next) = self.peek_byte() else {
                                    break;
                                };
                                if !is_octal_digit(next) {
                                    break;
                                }
                                octal.push(self.bump()?);
                            }
                            let value = std::str::from_utf8(&octal)
                                .ok()
                                .and_then(|digits| u16::from_str_radix(digits, 8).ok())
                                .map_or(0, |octal_value| octal_value & 0x00ff);
                            let value = u8::try_from(value).expect("octal value is masked to u8");
                            content.push(value);
                        }
                        _ => content.push(escaped),
                    }
                }
                _ => content.push(byte),
            }
        }

        self.content_error("unterminated literal string")
    }

    pub(super) fn parse_array(&mut self) -> Result<Object, ExtractError> {
        self.expect_byte(b'[')?;
        let mut values = Vec::new();
        loop {
            self.skip_ws_and_comments()?;
            if self.peek_byte() == Some(b']') {
                self.bump()?;
                return Ok(Object::Array(values));
            }
            values.push(self.parse_object()?);
        }
    }

    pub(super) fn parse_dictionary(&mut self) -> Result<Object, ExtractError> {
        self.expect_byte(b'<')?;
        self.expect_byte(b'<')?;
        let mut dict = Dictionary::new();

        loop {
            self.skip_ws_and_comments()?;
            if self.starts_with(b">>") {
                self.expect_byte(b'>')?;
                self.expect_byte(b'>')?;
                return Ok(Object::Dictionary(dict));
            }

            let key = self.parse_name_bytes()?;
            self.skip_ws_and_comments()?;
            let value = self.parse_object()?;
            dict.set(key, value);
        }
    }

    pub(super) fn parse_hex_string(&mut self) -> Result<Object, ExtractError> {
        self.expect_byte(b'<')?;
        let mut bytes = Vec::new();
        let mut high_nibble: Option<u8> = None;

        loop {
            let Some(byte) = self.peek_byte() else {
                return self.content_error("unterminated hexadecimal string");
            };

            if byte == b'>' {
                self.bump()?;
                if let Some(high) = high_nibble {
                    bytes.push(high << 4);
                }
                return Ok(Object::String(bytes, StringFormat::Hexadecimal));
            }

            if is_whitespace(byte) {
                self.bump()?;
                continue;
            }

            let Some(nibble) = hex_nibble(byte) else {
                return self.content_error("invalid hex nibble in hexadecimal string");
            };
            self.bump()?;
            if let Some(high) = high_nibble.take() {
                bytes.push((high << 4) | nibble);
            } else {
                high_nibble = Some(nibble);
            }
        }
    }

    pub(super) fn parse_number(&mut self) -> Result<Object, ExtractError> {
        let start = self.cursor;
        let mut saw_dot = false;
        let mut saw_digit = false;

        if matches!(self.peek_byte(), Some(b'+' | b'-')) {
            self.bump()?;
        }

        while let Some(byte) = self.peek_byte() {
            match byte {
                b'0'..=b'9' => {
                    saw_digit = true;
                    self.bump()?;
                }
                b'.' if !saw_dot => {
                    saw_dot = true;
                    self.bump()?;
                }
                _ => break,
            }
        }

        if !saw_digit {
            return self.content_error("invalid numeric token in content stream");
        }

        let token = std::str::from_utf8(&self.bytes[start..self.cursor]).map_err(|_| {
            ExtractError::ContentDecode {
                reason: format!("invalid UTF-8 in numeric token at byte offset {start}"),
            }
        })?;

        if saw_dot {
            let value: f32 = token.parse().map_err(|_| ExtractError::ContentDecode {
                reason: format!("invalid real token `{token}` at byte offset {start}"),
            })?;
            Ok(Object::Real(value))
        } else {
            let value: i64 = token.parse().map_err(|_| ExtractError::ContentDecode {
                reason: format!("invalid integer token `{token}` at byte offset {start}"),
            })?;
            Ok(Object::Integer(value))
        }
    }
}

fn is_octal_digit(byte: u8) -> bool {
    matches!(byte, b'0'..=b'7')
}
