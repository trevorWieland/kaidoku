use serde::{Deserialize, Deserializer, Serialize};
use std::num::NonZeroU32;
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq)]
pub enum ValidationError {
    #[error("bbox coordinates must be finite")]
    NonFiniteCoordinate,
    #[error("bbox width and height must be >= 0")]
    NegativeDimension,
    #[error("page number must be >= 1")]
    InvalidPageNumber,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PageNumber(NonZeroU32);

impl PageNumber {
    pub fn new(value: u32) -> Result<Self, ValidationError> {
        let Some(number) = NonZeroU32::new(value) else {
            return Err(ValidationError::InvalidPageNumber);
        };
        Ok(Self(number))
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0.get()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct BBox {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl<'de> Deserialize<'de> for BBox {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawBBox {
            x: f64,
            y: f64,
            width: f64,
            height: f64,
        }

        let raw = RawBBox::deserialize(deserializer)?;
        Self::new(raw.x, raw.y, raw.width, raw.height).map_err(serde::de::Error::custom)
    }
}

impl BBox {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Result<Self, ValidationError> {
        if !x.is_finite() || !y.is_finite() || !width.is_finite() || !height.is_finite() {
            return Err(ValidationError::NonFiniteCoordinate);
        }
        if width < 0.0 || height < 0.0 {
            return Err(ValidationError::NegativeDimension);
        }
        Ok(Self {
            x,
            y,
            width,
            height,
        })
    }

    #[must_use]
    pub const fn x(self) -> f64 {
        self.x
    }

    #[must_use]
    pub const fn y(self) -> f64 {
        self.y
    }

    #[must_use]
    pub const fn width(self) -> f64 {
        self.width
    }

    #[must_use]
    pub const fn height(self) -> f64 {
        self.height
    }

    #[must_use]
    pub fn quantized(self, precision: u8) -> Self {
        Self {
            x: quantize(self.x, precision),
            y: quantize(self.y, precision),
            width: quantize(self.width, precision),
            height: quantize(self.height, precision),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ElementKind {
    Char,
    Span,
    Image,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CharPayload {
    pub text: String,
    pub font_name: Option<String>,
    pub font_size: f64,
    pub char_index: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpanPayload {
    pub text: String,
    pub font_name: Option<String>,
    pub font_size: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImagePayload {
    pub name: String,
    pub width_px: u32,
    pub height_px: u32,
    pub color_space: Option<String>,
    pub bits_per_component: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "payload_type", rename_all = "snake_case")]
pub enum RawPayload {
    Char(CharPayload),
    Span(SpanPayload),
    Image(ImagePayload),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct SourceRef {
    page_number: PageNumber,
    page_object_number: u32,
    page_object_generation: u16,
    stream_index: u32,
    operation_index: u32,
    element_index: u32,
}

impl<'de> Deserialize<'de> for SourceRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawSourceRef {
            page_number: u32,
            page_object_number: u32,
            page_object_generation: u16,
            stream_index: u32,
            operation_index: u32,
            element_index: u32,
        }

        let raw = RawSourceRef::deserialize(deserializer)?;
        Self::new(
            raw.page_number,
            raw.page_object_number,
            raw.page_object_generation,
            raw.stream_index,
            raw.operation_index,
            raw.element_index,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl SourceRef {
    pub fn new(
        page_number: u32,
        page_object_number: u32,
        page_object_generation: u16,
        stream_index: u32,
        operation_index: u32,
        element_index: u32,
    ) -> Result<Self, ValidationError> {
        Ok(Self {
            page_number: PageNumber::new(page_number)?,
            page_object_number,
            page_object_generation,
            stream_index,
            operation_index,
            element_index,
        })
    }

    #[must_use]
    pub const fn page_number(self) -> PageNumber {
        self.page_number
    }

    #[must_use]
    pub const fn page_object_number(self) -> u32 {
        self.page_object_number
    }

    #[must_use]
    pub const fn page_object_generation(self) -> u16 {
        self.page_object_generation
    }

    #[must_use]
    pub const fn stream_index(self) -> u32 {
        self.stream_index
    }

    #[must_use]
    pub const fn operation_index(self) -> u32 {
        self.operation_index
    }

    #[must_use]
    pub const fn element_index(self) -> u32 {
        self.element_index
    }

    #[must_use]
    pub fn stable_key(self) -> String {
        format!(
            "p{}-o{}-s{}-i{}",
            self.page_number.get(),
            self.operation_index,
            self.stream_index,
            self.element_index
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawElement {
    kind: ElementKind,
    bbox: BBox,
    payload: RawPayload,
    source_ref: SourceRef,
}

impl RawElement {
    #[must_use]
    pub fn new(kind: ElementKind, bbox: BBox, payload: RawPayload, source_ref: SourceRef) -> Self {
        Self {
            kind,
            bbox,
            payload,
            source_ref,
        }
    }

    #[must_use]
    pub const fn kind(&self) -> ElementKind {
        self.kind
    }

    #[must_use]
    pub const fn bbox(&self) -> BBox {
        self.bbox
    }

    #[must_use]
    pub const fn source_ref(&self) -> SourceRef {
        self.source_ref
    }

    #[must_use]
    pub const fn payload(&self) -> &RawPayload {
        &self.payload
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtractionPage {
    pub page_number: PageNumber,
    pub width: f64,
    pub height: f64,
    pub elements: Vec<RawElement>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractionSource {
    pub backend: String,
    pub input_sha256: String,
    pub input_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtractionDocument {
    pub schema_version: String,
    pub source: ExtractionSource,
    pub pages: Vec<ExtractionPage>,
}

fn quantize(value: f64, precision: u8) -> f64 {
    let scale = 10_f64.powi(i32::from(precision));
    (value * scale).round() / scale
}

#[cfg(test)]
mod tests {
    use super::{BBox, SourceRef, ValidationError};

    #[test]
    fn bbox_rejects_invalid_values() {
        let invalid_width = BBox::new(0.0, 0.0, -1.0, 1.0);
        assert_eq!(invalid_width, Err(ValidationError::NegativeDimension));

        let invalid_nan = BBox::new(f64::NAN, 0.0, 1.0, 1.0);
        assert_eq!(invalid_nan, Err(ValidationError::NonFiniteCoordinate));
    }

    #[test]
    fn source_ref_stable_key_is_deterministic() {
        let source_ref = SourceRef::new(1, 12, 0, 0, 9, 3);
        assert!(source_ref.is_ok());

        let Ok(source_ref) = source_ref else { return };
        assert_eq!(source_ref.stable_key(), "p1-o9-s0-i3");
    }
}
