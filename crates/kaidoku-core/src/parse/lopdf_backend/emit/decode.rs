use super::{ExtractionControl, ExtractionStage};
use crate::{ExtractError, PageNumber};
use flate2::read::{DeflateDecoder, ZlibDecoder};
use lopdf::{Object, Stream};
use std::io::Read;
use weezl::{BitOrder, LzwStatus, decode::Decoder as LzwDecoder};

mod ascii85;
mod predictor;
use ascii85::decode_ascii85_bounded;
use predictor::{PredictorParams, apply_predictor};

pub(crate) fn decode_content_stream_bounded(
    stream: &Stream,
    page_number: PageNumber,
    stream_index: u32,
    stream_byte_limit: usize,
    control: &ExtractionControl,
) -> Result<Vec<u8>, ExtractError> {
    control.checkpoint(ExtractionStage::DecodeContentStreamStart, Some(page_number))?;

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

    for (index, filter_name) in filters.iter().enumerate() {
        control.checkpoint(
            ExtractionStage::DecodeContentStreamFilter,
            Some(page_number),
        )?;

        let filter_id =
            FilterId::from_name(filter_name).ok_or_else(|| ExtractError::ContentDecode {
                reason: format!(
                    "unsupported content stream filter {}",
                    String::from_utf8_lossy(filter_name)
                ),
            })?;
        let params = decode_params.and_then(|value| decode_params_for_filter(value, index));
        current = decode_filter_bounded(
            filter_id,
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
    filter: FilterId,
    input: &[u8],
    decode_params: Option<&lopdf::Dictionary>,
    page_number: PageNumber,
    stream_index: u32,
    limit: usize,
    control: &ExtractionControl,
) -> Result<Vec<u8>, ExtractError> {
    let decoded = match filter {
        FilterId::Flate => decode_zlib_bounded(input, page_number, stream_index, limit, control)?,
        FilterId::Lzw => decode_lzw_bounded(
            input,
            decode_params,
            page_number,
            stream_index,
            limit,
            control,
        )?,
        FilterId::Ascii85 => {
            decode_ascii85_bounded(input, page_number, stream_index, limit, control)?
        }
        FilterId::AsciiHex => {
            decode_ascii_hex_bounded(input, page_number, stream_index, limit, control)?
        }
        FilterId::RunLength => {
            decode_run_length_bounded(input, page_number, stream_index, limit, control)?
        }
    };

    let params = PredictorParams::from_dict(decode_params);
    if !params.is_identity() {
        return apply_predictor(&decoded, params, page_number, stream_index, limit, control);
    }
    Ok(decoded)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FilterId {
    Flate,
    Lzw,
    Ascii85,
    AsciiHex,
    RunLength,
}

impl FilterId {
    pub(super) fn from_name(name: &[u8]) -> Option<Self> {
        match name {
            b"FlateDecode" | b"Fl" => Some(Self::Flate),
            b"LZWDecode" | b"LZW" => Some(Self::Lzw),
            b"ASCII85Decode" | b"A85" => Some(Self::Ascii85),
            b"ASCIIHexDecode" | b"AHx" => Some(Self::AsciiHex),
            b"RunLengthDecode" | b"RL" => Some(Self::RunLength),
            _ => None,
        }
    }
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
        ExtractionStage::DecodeZlibChunk,
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
                ExtractionStage::DecodeDeflateFallbackChunk,
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
    control.checkpoint(ExtractionStage::DecodeLzwChunk, Some(page_number))?;

    let early_change = params
        .and_then(|dict| dict.get(b"EarlyChange").ok())
        .and_then(|value| value.as_i64().ok())
        != Some(0);

    let mut decoder = if early_change {
        LzwDecoder::with_tiff_size_switch(BitOrder::Msb, MIN_BITS - 1)
    } else {
        LzwDecoder::new(BitOrder::Msb, MIN_BITS - 1)
    };

    let mut remaining = input;
    let mut output = Vec::new();
    let mut chunk = [0_u8; 8 * 1024];

    loop {
        control.checkpoint(ExtractionStage::DecodeLzwChunk, Some(page_number))?;
        let result = decoder.decode_bytes(remaining, &mut chunk);
        remaining = remaining.get(result.consumed_in..).unwrap_or_default();

        if result.consumed_out > 0 {
            push_with_limit(
                &mut output,
                &chunk[..result.consumed_out],
                limit,
                page_number,
                stream_index,
            )?;
        }

        match result.status {
            Ok(LzwStatus::Done) => break,
            Ok(LzwStatus::Ok) => {}
            Ok(LzwStatus::NoProgress) => {
                return Err(ExtractError::ContentDecode {
                    reason: "LZW stream ended before explicit end marker".to_string(),
                });
            }
            Err(error) => {
                return Err(ExtractError::ContentDecode {
                    reason: error.to_string(),
                });
            }
        }

        if result.consumed_in == 0 && result.consumed_out == 0 {
            return Err(ExtractError::ContentDecode {
                reason: "LZW decoder made no progress".to_string(),
            });
        }
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
        control.checkpoint(ExtractionStage::DecodeAsciiHex, Some(page_number))?;

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
        control.checkpoint(ExtractionStage::DecodeRunLength, Some(page_number))?;

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
    checkpoint_stage: ExtractionStage,
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

#[cfg(test)]
mod tests;
