use super::{ExtractionControl, ExtractionStage, push_with_limit};
use crate::{ExtractError, PageNumber};

/// Resolved `DecodeParms` relevant to the predictor pipeline.
///
/// Captured once per filter application so the predictor pass is decoupled
/// from lopdf dictionary parsing. A predictor of `1` (identity) is encoded
/// via [`PredictorParams::is_identity`], which lets the caller short-circuit.
#[derive(Debug, Clone, Copy)]
pub(super) struct PredictorParams {
    predictor: i64,
    columns: usize,
    colors: usize,
    bits_per_component: usize,
}

const DEFAULT_COLUMNS: usize = 1;
const DEFAULT_COLORS: usize = 1;
const DEFAULT_BPC: usize = 8;
const MAX_COLUMNS: usize = 1 << 24;
const MAX_COLORS: usize = 32;
const MAX_BITS_PER_COMPONENT: usize = 16;

impl PredictorParams {
    pub(super) fn from_dict(params: Option<&lopdf::Dictionary>) -> Self {
        let predictor = params
            .and_then(|dict| dict.get(b"Predictor").ok())
            .and_then(|value| value.as_i64().ok())
            .unwrap_or(1);
        let columns = params
            .and_then(|dict| dict.get(b"Columns").ok())
            .and_then(|value| value.as_i64().ok())
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(DEFAULT_COLUMNS);
        let colors = params
            .and_then(|dict| dict.get(b"Colors").ok())
            .and_then(|value| value.as_i64().ok())
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(DEFAULT_COLORS);
        let bits_per_component = params
            .and_then(|dict| dict.get(b"BitsPerComponent").ok())
            .and_then(|value| value.as_i64().ok())
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(DEFAULT_BPC);

        Self {
            predictor,
            columns,
            colors,
            bits_per_component,
        }
    }

    pub(super) const fn is_identity(&self) -> bool {
        self.predictor == 1
    }
}

pub(super) fn apply_predictor(
    data: &[u8],
    params: PredictorParams,
    page_number: PageNumber,
    stream_index: u32,
    limit: usize,
    control: &ExtractionControl,
) -> Result<Vec<u8>, ExtractError> {
    control.checkpoint(ExtractionStage::DecodePredictor, Some(page_number))?;

    if params.columns == 0 || params.columns > MAX_COLUMNS {
        return Err(ExtractError::ContentDecode {
            reason: format!(
                "predictor Columns {} out of supported range",
                params.columns
            ),
        });
    }
    if params.colors == 0 || params.colors > MAX_COLORS {
        return Err(ExtractError::ContentDecode {
            reason: format!("predictor Colors {} out of supported range", params.colors),
        });
    }
    if params.bits_per_component == 0 || params.bits_per_component > MAX_BITS_PER_COMPONENT {
        return Err(ExtractError::ContentDecode {
            reason: format!(
                "predictor BitsPerComponent {} out of supported range",
                params.bits_per_component
            ),
        });
    }

    let pass = PredictorPass {
        page_number,
        stream_index,
        limit,
        control,
        params,
        bytes_per_pixel: pixel_stride_bytes(params.colors, params.bits_per_component)?,
        row_data_bytes: row_data_bytes(params.columns, params.colors, params.bits_per_component)?,
    };

    match params.predictor {
        1 => Ok(data.to_vec()),
        2 => pass.apply_tiff(data),
        10..=15 => pass.apply_png(data),
        other => Err(ExtractError::ContentDecode {
            reason: format!("unsupported predictor code {other}"),
        }),
    }
}

/// Per-application bundle that collapses the predictor row-loop signature.
/// Keeping all of these inline at every call site previously blew past
/// clippy's argument-count lint; capturing them once makes the predictor
/// loops read linearly.
struct PredictorPass<'a> {
    page_number: PageNumber,
    stream_index: u32,
    limit: usize,
    control: &'a ExtractionControl,
    params: PredictorParams,
    bytes_per_pixel: usize,
    row_data_bytes: usize,
}

fn pixel_stride_bytes(colors: usize, bits_per_component: usize) -> Result<usize, ExtractError> {
    let bits =
        colors
            .checked_mul(bits_per_component)
            .ok_or_else(|| ExtractError::ContentDecode {
                reason: "predictor pixel-stride overflow".to_string(),
            })?;
    Ok(bits.div_ceil(8).max(1))
}

fn row_data_bytes(
    columns: usize,
    colors: usize,
    bits_per_component: usize,
) -> Result<usize, ExtractError> {
    let total_bits = columns
        .checked_mul(colors)
        .and_then(|value| value.checked_mul(bits_per_component))
        .ok_or_else(|| ExtractError::ContentDecode {
            reason: "predictor row-width overflow".to_string(),
        })?;
    Ok(total_bits.div_ceil(8))
}

impl PredictorPass<'_> {
    fn apply_tiff(&self, data: &[u8]) -> Result<Vec<u8>, ExtractError> {
        if self.row_data_bytes == 0 {
            return Ok(Vec::new());
        }
        if data.len() % self.row_data_bytes != 0 {
            return Err(ExtractError::ContentDecode {
                reason: "TIFF predictor row length does not divide payload".to_string(),
            });
        }

        let mut output = Vec::with_capacity(data.len());
        for row in data.chunks_exact(self.row_data_bytes) {
            self.control
                .checkpoint(ExtractionStage::DecodePredictor, Some(self.page_number))?;
            let mut decoded_row = vec![0_u8; self.row_data_bytes];
            for column in 0..self.row_data_bytes {
                let left = column
                    .checked_sub(self.bytes_per_pixel)
                    .map_or(0, |idx| decoded_row[idx]);
                decoded_row[column] = row[column].wrapping_add(left);
            }
            push_with_limit(
                &mut output,
                &decoded_row,
                self.limit,
                self.page_number,
                self.stream_index,
            )?;
        }
        Ok(output)
    }

    fn apply_png(&self, data: &[u8]) -> Result<Vec<u8>, ExtractError> {
        let row_stride =
            self.row_data_bytes
                .checked_add(1)
                .ok_or_else(|| ExtractError::ContentDecode {
                    reason: "predictor row-stride overflow".to_string(),
                })?;
        if data.is_empty() {
            return Ok(Vec::new());
        }
        if data.len() % row_stride != 0 {
            return Err(ExtractError::ContentDecode {
                reason: format!(
                    "PNG predictor payload {} is not a multiple of row stride {row_stride}",
                    data.len()
                ),
            });
        }

        let fixed_tag = png_fixed_tag(self.params.predictor);
        let mut previous_row = vec![0_u8; self.row_data_bytes];
        let mut output = Vec::with_capacity(data.len() - data.len() / row_stride);

        for row in data.chunks_exact(row_stride) {
            self.control
                .checkpoint(ExtractionStage::DecodePredictor, Some(self.page_number))?;
            let (tag_byte, payload) =
                row.split_first()
                    .ok_or_else(|| ExtractError::ContentDecode {
                        reason: "PNG predictor row missing tag byte".to_string(),
                    })?;

            let tag = fixed_tag.unwrap_or(*tag_byte);
            if tag > 4 {
                return Err(ExtractError::ContentDecode {
                    reason: format!("PNG predictor row tag {tag} out of range 0..=4"),
                });
            }

            let mut decoded_row = vec![0_u8; self.row_data_bytes];
            for column in 0..self.row_data_bytes {
                let left = column
                    .checked_sub(self.bytes_per_pixel)
                    .map_or(0, |idx| decoded_row[idx]);
                let up = previous_row[column];
                let up_left = column
                    .checked_sub(self.bytes_per_pixel)
                    .map_or(0, |idx| previous_row[idx]);
                let raw = payload[column];
                decoded_row[column] = match tag {
                    0 => raw,
                    1 => raw.wrapping_add(left),
                    2 => raw.wrapping_add(up),
                    3 => {
                        let average = u8::try_from(u16::midpoint(u16::from(left), u16::from(up)))
                            .unwrap_or(0);
                        raw.wrapping_add(average)
                    }
                    4 => raw.wrapping_add(paeth_predictor(left, up, up_left)),
                    _ => {
                        return Err(ExtractError::InvariantViolation {
                            reason: "PNG predictor tag range already validated".to_string(),
                        });
                    }
                };
            }

            push_with_limit(
                &mut output,
                &decoded_row,
                self.limit,
                self.page_number,
                self.stream_index,
            )?;
            previous_row = decoded_row;
        }

        Ok(output)
    }
}

fn png_fixed_tag(predictor: i64) -> Option<u8> {
    // `15` (Optimum) and any fallthrough return `None` — each row then
    // carries its own tag byte, which the caller validates.
    match predictor {
        10 => Some(0),
        11 => Some(1),
        12 => Some(2),
        13 => Some(3),
        14 => Some(4),
        _ => None,
    }
}

fn paeth_predictor(left: u8, up: u8, up_left: u8) -> u8 {
    let left = i32::from(left);
    let up = i32::from(up);
    let up_left = i32::from(up_left);
    let p = left + up - up_left;
    let pa = (p - left).abs();
    let pb = (p - up).abs();
    let pc = (p - up_left).abs();
    let selected = if pa <= pb && pa <= pc {
        left
    } else if pb <= pc {
        up
    } else {
        up_left
    };
    u8::try_from(selected & 0xFF).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{PredictorParams, apply_predictor, paeth_predictor};
    use crate::PageNumber;
    use crate::parse::lopdf_backend::emit::ExtractionControl;

    #[test]
    fn identity_predictor_is_zero_cost() {
        assert!(PredictorParams::from_dict(None).is_identity());
    }

    #[test]
    fn png_up_predictor_roundtrips_simple_row() {
        let params = mock_params(12, 4, 1, 8);
        let page_number = PageNumber::new(1).expect("page");
        let control = ExtractionControl::new(10_000, None);
        // Encoded as PNG Up: first row tag=2 with raw values, second row tag=2
        // with deltas against the first row. Decoded output concatenates the
        // rows without the tag byte.
        let input = [2_u8, 0x10, 0x20, 0x30, 0x40, 2_u8, 0x01, 0x02, 0x03, 0x04];
        let decoded = apply_predictor(&input, params, page_number, 0, 1024, &control)
            .expect("predictor decode");
        assert_eq!(
            decoded,
            vec![0x10, 0x20, 0x30, 0x40, 0x11, 0x22, 0x33, 0x44]
        );
    }

    #[test]
    fn invalid_png_tag_is_rejected() {
        let params = mock_params(15, 4, 1, 8);
        let page_number = PageNumber::new(1).expect("page");
        let control = ExtractionControl::new(10_000, None);
        let input = [9_u8, 0, 0, 0, 0];
        let err = apply_predictor(&input, params, page_number, 0, 1024, &control)
            .expect_err("invalid tag");
        assert!(err.to_string().contains("out of range"));
    }

    #[test]
    fn malformed_row_length_is_rejected() {
        let params = mock_params(12, 4, 1, 8);
        let page_number = PageNumber::new(1).expect("page");
        let control = ExtractionControl::new(10_000, None);
        let input = [2_u8, 0x10, 0x20, 0x30];
        let err = apply_predictor(&input, params, page_number, 0, 1024, &control)
            .expect_err("row length mismatch");
        assert!(err.to_string().contains("not a multiple"));
    }

    #[test]
    fn predictor_honors_stream_limit() {
        let params = mock_params(12, 4, 1, 8);
        let page_number = PageNumber::new(1).expect("page");
        let control = ExtractionControl::new(10_000, None);
        let input = [2_u8, 0x10, 0x20, 0x30, 0x40, 2_u8, 0x01, 0x02, 0x03, 0x04];
        let err = apply_predictor(&input, params, page_number, 0, 4, &control)
            .expect_err("limit exceeded");
        assert!(matches!(
            err,
            crate::ExtractError::ContentStreamDecodeLimitExceeded { .. }
        ));
    }

    #[test]
    fn paeth_selects_neighbour_with_minimum_distance() {
        assert_eq!(paeth_predictor(10, 20, 5), 20);
        assert_eq!(paeth_predictor(0, 0, 0), 0);
        assert_eq!(paeth_predictor(255, 0, 0), 255);
    }

    fn mock_params(predictor: i64, columns: usize, colors: usize, bpc: usize) -> PredictorParams {
        // Reconstructs a PredictorParams via the public constructor path so we
        // don't rely on struct field visibility.
        let mut dict = lopdf::Dictionary::new();
        dict.set(b"Predictor", predictor);
        dict.set(b"Columns", i64::try_from(columns).unwrap_or(1));
        dict.set(b"Colors", i64::try_from(colors).unwrap_or(1));
        dict.set(b"BitsPerComponent", i64::try_from(bpc).unwrap_or(8));
        PredictorParams::from_dict(Some(&dict))
    }
}
