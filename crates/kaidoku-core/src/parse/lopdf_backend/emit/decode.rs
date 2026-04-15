use super::ExtractionControl;
use crate::{ExtractError, PageNumber};
use flate2::read::{DeflateDecoder, ZlibDecoder};
use lopdf::{Object, Stream};
use std::io::{self, Read, Write};
use weezl::{BitOrder, decode::Decoder as LzwDecoder};

pub(crate) fn decode_content_stream_bounded(
    stream: &Stream,
    page_number: PageNumber,
    stream_index: u32,
    stream_byte_limit: usize,
    control: &ExtractionControl,
) -> Result<Vec<u8>, ExtractError> {
    control.checkpoint("decode_content_stream_start", Some(page_number))?;

    let filters = if stream.dict.get(b"Filter").is_ok() {
        stream
            .filters()
            .map_err(|error| ExtractError::ContentDecode {
                reason: error.to_string(),
            })?
    } else {
        Vec::new()
    };

    if filters.is_empty() {
        if stream.content.len() > stream_byte_limit {
            return Err(ExtractError::ContentStreamDecodeLimitExceeded {
                page_number: page_number.get(),
                stream_index,
                limit_bytes: stream_byte_limit,
                actual_bytes: stream.content.len(),
            });
        }
        return Ok(stream.content.clone());
    }

    let decode_params = stream.dict.get(b"DecodeParms").ok();
    let mut current = stream.content.clone();

    for (index, filter) in filters.iter().enumerate() {
        control.checkpoint("decode_content_stream_filter", Some(page_number))?;

        let params = decode_params.and_then(|value| decode_params_for_filter(value, index));
        current = decode_filter_bounded(
            filter,
            &current,
            params,
            page_number,
            stream_index,
            stream_byte_limit,
            control,
        )?;
    }

    Ok(current)
}

fn decode_params_for_filter(value: &Object, index: usize) -> Option<&lopdf::Dictionary> {
    match value {
        Object::Dictionary(dict) => Some(dict),
        Object::Array(values) => values.get(index)?.as_dict().ok(),
        _ => None,
    }
}

fn decode_filter_bounded(
    filter: &[u8],
    input: &[u8],
    decode_params: Option<&lopdf::Dictionary>,
    page_number: PageNumber,
    stream_index: u32,
    limit: usize,
    control: &ExtractionControl,
) -> Result<Vec<u8>, ExtractError> {
    if predictor_value(decode_params).unwrap_or(1) > 1 {
        return Err(ExtractError::ContentDecode {
            reason: "DecodeParms Predictor is not supported for content streams".to_string(),
        });
    }

    match filter {
        b"FlateDecode" | b"Fl" => {
            decode_zlib_bounded(input, page_number, stream_index, limit, control)
        }
        b"LZWDecode" | b"LZW" => decode_lzw_bounded(
            input,
            decode_params,
            page_number,
            stream_index,
            limit,
            control,
        ),
        b"ASCII85Decode" | b"A85" => {
            decode_ascii85_bounded(input, page_number, stream_index, limit, control)
        }
        b"ASCIIHexDecode" | b"AHx" => {
            decode_ascii_hex_bounded(input, page_number, stream_index, limit, control)
        }
        b"RunLengthDecode" | b"RL" => {
            decode_run_length_bounded(input, page_number, stream_index, limit, control)
        }
        _ => Err(ExtractError::ContentDecode {
            reason: format!(
                "unsupported content stream filter {}",
                String::from_utf8_lossy(filter)
            ),
        }),
    }
}

fn predictor_value(params: Option<&lopdf::Dictionary>) -> Option<i64> {
    params
        .and_then(|dict| dict.get(b"Predictor").ok())
        .and_then(|value| value.as_i64().ok())
}

fn decode_zlib_bounded(
    input: &[u8],
    page_number: PageNumber,
    stream_index: u32,
    limit: usize,
    control: &ExtractionControl,
) -> Result<Vec<u8>, ExtractError> {
    if input.is_empty() {
        return Ok(Vec::new());
    }

    let mut decoder = ZlibDecoder::new(input);
    match read_to_limit(
        &mut decoder,
        limit,
        page_number,
        stream_index,
        "decode_zlib",
        control,
    ) {
        Ok(output) => Ok(output),
        Err(zlib_error) => {
            let mut fallback_decoder = DeflateDecoder::new(input.get(2..).unwrap_or_default());
            read_to_limit(
                &mut fallback_decoder,
                limit,
                page_number,
                stream_index,
                "decode_deflate_fallback",
                control,
            )
            .map_err(|_| zlib_error)
        }
    }
}

fn decode_lzw_bounded(
    input: &[u8],
    params: Option<&lopdf::Dictionary>,
    page_number: PageNumber,
    stream_index: u32,
    limit: usize,
    control: &ExtractionControl,
) -> Result<Vec<u8>, ExtractError> {
    const MIN_BITS: u8 = 9;
    control.checkpoint("decode_lzw", Some(page_number))?;

    let early_change = params
        .and_then(|dict| dict.get(b"EarlyChange").ok())
        .and_then(|value| value.as_i64().ok())
        != Some(0);

    let mut decoder = if early_change {
        LzwDecoder::with_tiff_size_switch(BitOrder::Msb, MIN_BITS - 1)
    } else {
        LzwDecoder::new(BitOrder::Msb, MIN_BITS - 1)
    };

    let mut writer = LimitedWriter::new(limit);
    let result = decoder.into_stream(&mut writer).decode_all(input);

    if writer.overflowed() {
        return Err(ExtractError::ContentStreamDecodeLimitExceeded {
            page_number: page_number.get(),
            stream_index,
            limit_bytes: limit,
            actual_bytes: writer.attempted_len(),
        });
    }

    if let Err(error) = result.status {
        return Err(ExtractError::ContentDecode {
            reason: error.to_string(),
        });
    }

    Ok(writer.into_inner())
}

fn decode_ascii85_bounded(
    input: &[u8],
    page_number: PageNumber,
    stream_index: u32,
    limit: usize,
    control: &ExtractionControl,
) -> Result<Vec<u8>, ExtractError> {
    let mut output = Vec::new();
    let mut buffer: u32 = 0;
    let mut count = 0u8;

    let input_no_eod = if input.len() >= 2 && &input[input.len() - 2..] == b"~>" {
        &input[..input.len() - 2]
    } else {
        input
    };

    for &character in input_no_eod {
        control.checkpoint("decode_ascii85", Some(page_number))?;

        if character == b'z' {
            if count != 0 {
                return Err(ExtractError::ContentDecode {
                    reason: "invalid ASCII85 stream".to_string(),
                });
            }
            push_with_limit(&mut output, &[0, 0, 0, 0], limit, page_number, stream_index)?;
            continue;
        }

        if character.is_ascii_whitespace() {
            continue;
        }

        if !(b'!'..=b'u').contains(&character) {
            break;
        }

        buffer = buffer
            .checked_mul(85)
            .and_then(|value| value.checked_add(u32::from(character - b'!')))
            .ok_or(ExtractError::ContentDecode {
                reason: "ASCII85 overflow".to_string(),
            })?;

        count = count.saturating_add(1);

        if count == 5 {
            push_with_limit(
                &mut output,
                &buffer.to_be_bytes(),
                limit,
                page_number,
                stream_index,
            )?;
            buffer = 0;
            count = 0;
        }
    }

    if count > 0 {
        for _ in count..5 {
            buffer = buffer
                .checked_mul(85)
                .and_then(|value| value.checked_add(84))
                .ok_or(ExtractError::ContentDecode {
                    reason: "ASCII85 tail overflow".to_string(),
                })?;
        }

        let bytes = buffer.to_be_bytes();
        let take = usize::from(count.saturating_sub(1));
        push_with_limit(
            &mut output,
            &bytes[..take],
            limit,
            page_number,
            stream_index,
        )?;
    }

    Ok(output)
}

fn decode_ascii_hex_bounded(
    input: &[u8],
    page_number: PageNumber,
    stream_index: u32,
    limit: usize,
    control: &ExtractionControl,
) -> Result<Vec<u8>, ExtractError> {
    let mut output = Vec::new();
    let mut upper_nibble: Option<u8> = None;

    for &byte in input {
        control.checkpoint("decode_ascii_hex", Some(page_number))?;

        if byte == b'>' {
            break;
        }
        if byte.is_ascii_whitespace() {
            continue;
        }

        let nibble = ascii_hex_nibble(byte).ok_or(ExtractError::ContentDecode {
            reason: "invalid ASCIIHex stream".to_string(),
        })?;

        if let Some(upper) = upper_nibble.take() {
            push_with_limit(
                &mut output,
                &[(upper << 4) | nibble],
                limit,
                page_number,
                stream_index,
            )?;
        } else {
            upper_nibble = Some(nibble);
        }
    }

    if let Some(upper) = upper_nibble {
        push_with_limit(&mut output, &[upper << 4], limit, page_number, stream_index)?;
    }

    Ok(output)
}

fn decode_run_length_bounded(
    input: &[u8],
    page_number: PageNumber,
    stream_index: u32,
    limit: usize,
    control: &ExtractionControl,
) -> Result<Vec<u8>, ExtractError> {
    let mut output = Vec::new();
    let mut cursor = 0usize;

    while cursor < input.len() {
        control.checkpoint("decode_run_length", Some(page_number))?;

        let length_byte = input[cursor];
        cursor = cursor.saturating_add(1);

        if length_byte == 128 {
            break;
        }

        if length_byte <= 127 {
            let run_length = usize::from(length_byte).saturating_add(1);
            let end = cursor.saturating_add(run_length);
            let bytes = input.get(cursor..end).ok_or(ExtractError::ContentDecode {
                reason: "truncated RunLength literal run".to_string(),
            })?;
            push_with_limit(&mut output, bytes, limit, page_number, stream_index)?;
            cursor = end;
            continue;
        }

        let repeat = usize::from(257_u16.saturating_sub(u16::from(length_byte)));
        let value = *input.get(cursor).ok_or(ExtractError::ContentDecode {
            reason: "truncated RunLength repeat run".to_string(),
        })?;
        cursor = cursor.saturating_add(1);

        for _ in 0..repeat {
            push_with_limit(&mut output, &[value], limit, page_number, stream_index)?;
        }
    }

    Ok(output)
}

fn ascii_hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn push_with_limit(
    output: &mut Vec<u8>,
    bytes: &[u8],
    limit: usize,
    page_number: PageNumber,
    stream_index: u32,
) -> Result<(), ExtractError> {
    let attempted = output.len().saturating_add(bytes.len());
    if attempted > limit {
        return Err(ExtractError::ContentStreamDecodeLimitExceeded {
            page_number: page_number.get(),
            stream_index,
            limit_bytes: limit,
            actual_bytes: attempted,
        });
    }
    output.extend_from_slice(bytes);
    Ok(())
}

fn read_to_limit<R: Read>(
    reader: &mut R,
    limit: usize,
    page_number: PageNumber,
    stream_index: u32,
    checkpoint_stage: &'static str,
    control: &ExtractionControl,
) -> Result<Vec<u8>, ExtractError> {
    let mut output = Vec::new();
    let mut chunk = [0_u8; 8 * 1024];

    loop {
        control.checkpoint(checkpoint_stage, Some(page_number))?;

        let read = reader
            .read(&mut chunk)
            .map_err(|error| ExtractError::ContentDecode {
                reason: error.to_string(),
            })?;

        if read == 0 {
            break;
        }

        let attempted = output.len().saturating_add(read);
        if attempted > limit {
            return Err(ExtractError::ContentStreamDecodeLimitExceeded {
                page_number: page_number.get(),
                stream_index,
                limit_bytes: limit,
                actual_bytes: attempted,
            });
        }

        output.extend_from_slice(&chunk[..read]);
    }

    Ok(output)
}

#[derive(Debug)]
struct LimitedWriter {
    limit: usize,
    attempted_len: usize,
    overflowed: bool,
    output: Vec<u8>,
}

impl LimitedWriter {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            attempted_len: 0,
            overflowed: false,
            output: Vec::new(),
        }
    }

    const fn overflowed(&self) -> bool {
        self.overflowed
    }

    const fn attempted_len(&self) -> usize {
        self.attempted_len
    }

    fn into_inner(self) -> Vec<u8> {
        self.output
    }
}

impl Write for LimitedWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.attempted_len = self.output.len().saturating_add(buf.len());
        if self.attempted_len > self.limit {
            self.overflowed = true;
            return Err(io::Error::other("decoded stream exceeded configured limit"));
        }

        self.output.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
