use super::{ExtractionControl, ExtractionStage, push_with_limit};
use crate::{ExtractError, PageNumber};

/// Strict ASCII85 decoder.
///
/// Pre-remediation this routine silently truncated on any invalid byte,
/// which allowed malformed streams to produce partial content. The decoder
/// now errors on invalid bytes, rejects incomplete final groups, and only
/// tolerates whitespace after the `~>` terminator — everything else is a
/// corruption signal.
pub(super) fn decode_ascii85_bounded(
    input: &[u8],
    page_number: PageNumber,
    stream_index: u32,
    limit: usize,
    control: &ExtractionControl,
) -> Result<Vec<u8>, ExtractError> {
    let mut output = Vec::new();
    let mut buffer: u32 = 0;
    let mut count = 0u8;
    let mut terminated = false;

    let mut cursor = 0usize;
    while cursor < input.len() {
        control.checkpoint(ExtractionStage::DecodeAscii85, Some(page_number))?;

        let character = input[cursor];
        cursor = cursor.saturating_add(1);

        if character == b'~' {
            if cursor >= input.len() || input[cursor] != b'>' {
                return Err(ExtractError::ContentDecode {
                    reason: "ASCII85 `~` not followed by `>`".to_string(),
                });
            }
            cursor = cursor.saturating_add(1);
            terminated = true;
            break;
        }

        if character == b'z' {
            if count != 0 {
                return Err(ExtractError::ContentDecode {
                    reason: "ASCII85 `z` shortcut appeared mid-group".to_string(),
                });
            }
            push_with_limit(&mut output, &[0, 0, 0, 0], limit, page_number, stream_index)?;
            continue;
        }

        if character.is_ascii_whitespace() {
            continue;
        }

        if !(b'!'..=b'u').contains(&character) {
            return Err(ExtractError::ContentDecode {
                reason: format!("ASCII85 invalid byte 0x{character:02x}"),
            });
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

    if terminated {
        while cursor < input.len() {
            let trailing = input[cursor];
            cursor = cursor.saturating_add(1);
            if !trailing.is_ascii_whitespace() {
                return Err(ExtractError::ContentDecode {
                    reason: format!("ASCII85 trailing byte 0x{trailing:02x} after `~>`"),
                });
            }
        }
    }

    if count == 1 {
        return Err(ExtractError::ContentDecode {
            reason: "ASCII85 incomplete final group (only 1 byte remaining)".to_string(),
        });
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
