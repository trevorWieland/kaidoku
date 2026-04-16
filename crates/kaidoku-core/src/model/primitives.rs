use serde::{Deserialize, Deserializer, Serialize, Serializer};
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
    #[error("font id must be >= 1")]
    InvalidFontId,
    #[error("schema identifier is invalid")]
    InvalidSchemaIdentifier,
    #[error("backend identifier is invalid")]
    InvalidBackendIdentifier,
    #[error("sha256 digest must be lowercase hex with exactly 64 characters")]
    InvalidSha256Digest,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FontId(NonZeroU32);

impl FontId {
    pub fn new(value: u32) -> Result<Self, ValidationError> {
        let Some(number) = NonZeroU32::new(value) else {
            return Err(ValidationError::InvalidFontId);
        };
        Ok(Self(number))
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0.get()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaIdentifier {
    Phase1V2,
}

impl SchemaIdentifier {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Phase1V2 => "kaidoku.phase1.v2",
        }
    }
}

impl Serialize for SchemaIdentifier {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for SchemaIdentifier {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        match value.as_str() {
            "kaidoku.phase1.v2" => Ok(Self::Phase1V2),
            _ => Err(serde::de::Error::custom(
                ValidationError::InvalidSchemaIdentifier,
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendIdentifier {
    Lopdf,
}

impl BackendIdentifier {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Lopdf => "lopdf",
        }
    }
}

impl Serialize for BackendIdentifier {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for BackendIdentifier {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        match value.as_str() {
            "lopdf" => Ok(Self::Lopdf),
            _ => Err(serde::de::Error::custom(
                ValidationError::InvalidBackendIdentifier,
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sha256Digest(String);

impl Sha256Digest {
    pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into();
        let valid = value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        if !valid {
            return Err(ValidationError::InvalidSha256Digest);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for Sha256Digest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Sha256Digest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
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
    pub font_id: Option<FontId>,
    pub font_size: f64,
    pub char_index: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpanPayload {
    pub text: String,
    pub font_id: Option<FontId>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FontDescriptor {
    pub id: FontId,
    pub name: String,
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
            "p{}-po{}g{}-o{}-s{}-i{}",
            self.page_number.get(),
            self.page_object_number,
            self.page_object_generation,
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
    pub backend: BackendIdentifier,
    pub input_sha256: Sha256Digest,
    pub input_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtractionDocument {
    pub schema_version: SchemaIdentifier,
    pub source: ExtractionSource,
    pub fonts: Vec<FontDescriptor>,
    pub pages: Vec<ExtractionPage>,
}

fn quantize(value: f64, precision: u8) -> f64 {
    let scale = 10_f64.powi(i32::from(precision));
    (value * scale).round() / scale
}

#[cfg(test)]
mod tests;
