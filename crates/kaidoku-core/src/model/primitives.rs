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
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RawElement {
    Char {
        bbox: BBox,
        source_ref: SourceRef,
        payload: CharPayload,
    },
    Span {
        bbox: BBox,
        source_ref: SourceRef,
        payload: SpanPayload,
    },
    Image {
        bbox: BBox,
        source_ref: SourceRef,
        payload: ImagePayload,
    },
}

impl RawElement {
    #[must_use]
    pub const fn char(bbox: BBox, source_ref: SourceRef, payload: CharPayload) -> Self {
        Self::Char {
            bbox,
            source_ref,
            payload,
        }
    }

    #[must_use]
    pub const fn span(bbox: BBox, source_ref: SourceRef, payload: SpanPayload) -> Self {
        Self::Span {
            bbox,
            source_ref,
            payload,
        }
    }

    #[must_use]
    pub const fn image(bbox: BBox, source_ref: SourceRef, payload: ImagePayload) -> Self {
        Self::Image {
            bbox,
            source_ref,
            payload,
        }
    }

    #[must_use]
    pub const fn bbox(&self) -> BBox {
        match self {
            Self::Char { bbox, .. } | Self::Span { bbox, .. } | Self::Image { bbox, .. } => *bbox,
        }
    }

    #[must_use]
    pub const fn source_ref(&self) -> SourceRef {
        match self {
            Self::Char { source_ref, .. }
            | Self::Span { source_ref, .. }
            | Self::Image { source_ref, .. } => *source_ref,
        }
    }

    #[must_use]
    pub const fn char_payload(&self) -> Option<&CharPayload> {
        match self {
            Self::Char { payload, .. } => Some(payload),
            Self::Span { .. } | Self::Image { .. } => None,
        }
    }

    #[must_use]
    pub const fn span_payload(&self) -> Option<&SpanPayload> {
        match self {
            Self::Span { payload, .. } => Some(payload),
            Self::Char { .. } | Self::Image { .. } => None,
        }
    }

    #[must_use]
    pub const fn image_payload(&self) -> Option<&ImagePayload> {
        match self {
            Self::Image { payload, .. } => Some(payload),
            Self::Char { .. } | Self::Span { .. } => None,
        }
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
    use super::{
        BBox, CharPayload, ImagePayload, RawElement, SourceRef, SpanPayload, ValidationError,
    };
    use serde_json::from_str;

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

    #[test]
    fn page_number_and_source_ref_deserialize_paths_are_validated() {
        let invalid_page = SourceRef::new(0, 1, 0, 0, 0, 0);
        assert_eq!(invalid_page, Err(ValidationError::InvalidPageNumber));

        let parsed = from_str::<SourceRef>(
            r#"{
                "page_number": 3,
                "page_object_number": 12,
                "page_object_generation": 0,
                "stream_index": 2,
                "operation_index": 7,
                "element_index": 4
            }"#,
        );
        assert!(parsed.is_ok());

        let Ok(parsed) = parsed else { return };
        assert_eq!(parsed.page_number().get(), 3);
        assert_eq!(parsed.page_object_number(), 12);
        assert_eq!(parsed.page_object_generation(), 0);
        assert_eq!(parsed.stream_index(), 2);
        assert_eq!(parsed.operation_index(), 7);
        assert_eq!(parsed.element_index(), 4);

        let invalid = from_str::<SourceRef>(
            r#"{
                "page_number": 0,
                "page_object_number": 12,
                "page_object_generation": 0,
                "stream_index": 2,
                "operation_index": 7,
                "element_index": 4
            }"#,
        );
        assert!(invalid.is_err());
    }

    #[test]
    fn bbox_deserialize_and_quantize_cover_accessors() {
        let parsed = from_str::<BBox>(r#"{"x":1.2349,"y":-0.0101,"width":4.4449,"height":9.9951}"#);
        assert!(parsed.is_ok());
        let Ok(parsed) = parsed else { return };

        let quantized = parsed.quantized(2);
        assert!((quantized.x() - 1.23).abs() < f64::EPSILON);
        assert!((quantized.y() + 0.01).abs() < f64::EPSILON);
        assert!((quantized.width() - 4.44).abs() < f64::EPSILON);
        assert!((quantized.height() - 10.0).abs() < f64::EPSILON);

        let invalid = from_str::<BBox>(r#"{"x":1.0,"y":2.0,"width":-1.0,"height":2.0}"#);
        assert!(invalid.is_err());
    }

    #[test]
    fn raw_element_variant_accessors_are_type_safe() {
        let bbox = BBox::new(0.0, 1.0, 2.0, 3.0);
        assert!(bbox.is_ok());
        let Ok(bbox) = bbox else { return };

        let source_ref = SourceRef::new(1, 9, 0, 2, 3, 4);
        assert!(source_ref.is_ok());
        let Ok(source_ref) = source_ref else { return };

        let char_element = RawElement::char(
            bbox,
            source_ref,
            CharPayload {
                text: "A".to_string(),
                font_name: Some("F1".to_string()),
                font_size: 12.0,
                char_index: 0,
            },
        );
        assert!(char_element.char_payload().is_some());
        assert!(char_element.span_payload().is_none());
        assert!(char_element.image_payload().is_none());
        assert_eq!(char_element.bbox(), bbox);
        assert_eq!(char_element.source_ref(), source_ref);

        let span_element = RawElement::span(
            bbox,
            source_ref,
            SpanPayload {
                text: "AB".to_string(),
                font_name: None,
                font_size: 10.0,
            },
        );
        assert!(span_element.char_payload().is_none());
        assert!(span_element.span_payload().is_some());
        assert!(span_element.image_payload().is_none());

        let image_element = RawElement::image(
            bbox,
            source_ref,
            ImagePayload {
                name: "Im1".to_string(),
                width_px: 16,
                height_px: 8,
                color_space: Some("DeviceRGB".to_string()),
                bits_per_component: Some(8),
            },
        );
        assert!(image_element.char_payload().is_none());
        assert!(image_element.span_payload().is_none());
        assert!(image_element.image_payload().is_some());
    }
}
