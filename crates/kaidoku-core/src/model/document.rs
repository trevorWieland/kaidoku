use super::primitives::{BBox, FontId, PageNumber, SourceRef, ValidationError};
use super::scalars::{NonNegativeFinite, PositiveFinite};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::num::NonZeroU8;

#[derive(Debug, Clone, PartialEq)]
pub struct CharPayload {
    text: String,
    font_id: Option<FontId>,
    font_size: PositiveFinite,
    char_index: u32,
    /// Number of Unicode codepoints the originating PDF glyph expanded to.
    ///
    /// The common case is `1` (one glyph → one codepoint). For ligatures and
    /// CMap-expanded glyphs this is `>1`, in which case every component char
    /// of the same glyph shares the glyph's bbox — the PDF did not give us
    /// per-component spatial resolution, so we surface that uncertainty
    /// explicitly instead of splitting the glyph's advance evenly.
    glyph_component_count: NonZeroU8,
}

impl CharPayload {
    pub fn new(
        text: impl Into<String>,
        font_id: Option<FontId>,
        font_size: f64,
        char_index: u32,
    ) -> Result<Self, ValidationError> {
        Self::with_glyph_component_count(text, font_id, font_size, char_index, 1)
    }

    pub fn with_glyph_component_count(
        text: impl Into<String>,
        font_id: Option<FontId>,
        font_size: f64,
        char_index: u32,
        glyph_component_count: u8,
    ) -> Result<Self, ValidationError> {
        let component_count = NonZeroU8::new(glyph_component_count)
            .ok_or(ValidationError::InvalidGlyphComponentCount)?;
        Ok(Self {
            text: text.into(),
            font_id,
            font_size: PositiveFinite::new(font_size)?,
            char_index,
            glyph_component_count: component_count,
        })
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub const fn font_id(&self) -> Option<FontId> {
        self.font_id
    }

    #[must_use]
    pub const fn font_size(&self) -> f64 {
        self.font_size.get()
    }

    #[must_use]
    pub const fn char_index(&self) -> u32 {
        self.char_index
    }

    #[must_use]
    pub const fn glyph_component_count(&self) -> u8 {
        self.glyph_component_count.get()
    }
}

fn is_one_glyph_component(count: NonZeroU8) -> bool {
    count.get() == 1
}

impl Serialize for CharPayload {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeStruct;
        let extra = usize::from(!is_one_glyph_component(self.glyph_component_count));
        let mut state = serializer.serialize_struct("CharPayload", 4 + extra)?;
        state.serialize_field("text", &self.text)?;
        state.serialize_field("font_id", &self.font_id)?;
        state.serialize_field("font_size", &self.font_size)?;
        state.serialize_field("char_index", &self.char_index)?;
        if !is_one_glyph_component(self.glyph_component_count) {
            state.serialize_field("glyph_component_count", &self.glyph_component_count.get())?;
        }
        state.end()
    }
}

impl<'de> Deserialize<'de> for CharPayload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            text: String,
            font_id: Option<FontId>,
            font_size: f64,
            char_index: u32,
            #[serde(default = "default_glyph_component_count")]
            glyph_component_count: u8,
        }
        fn default_glyph_component_count() -> u8 {
            1
        }
        let raw = Raw::deserialize(deserializer)?;
        Self::with_glyph_component_count(
            raw.text,
            raw.font_id,
            raw.font_size,
            raw.char_index,
            raw.glyph_component_count,
        )
        .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpanPayload {
    text: String,
    font_id: Option<FontId>,
    font_size: PositiveFinite,
}

impl SpanPayload {
    pub fn new(
        text: impl Into<String>,
        font_id: Option<FontId>,
        font_size: f64,
    ) -> Result<Self, ValidationError> {
        Ok(Self {
            text: text.into(),
            font_id,
            font_size: PositiveFinite::new(font_size)?,
        })
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub const fn font_id(&self) -> Option<FontId> {
        self.font_id
    }

    #[must_use]
    pub const fn font_size(&self) -> f64 {
        self.font_size.get()
    }
}

impl Serialize for SpanPayload {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("SpanPayload", 3)?;
        state.serialize_field("text", &self.text)?;
        state.serialize_field("font_id", &self.font_id)?;
        state.serialize_field("font_size", &self.font_size)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for SpanPayload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            text: String,
            font_id: Option<FontId>,
            font_size: f64,
        }
        let raw = Raw::deserialize(deserializer)?;
        Self::new(raw.text, raw.font_id, raw.font_size).map_err(serde::de::Error::custom)
    }
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

#[derive(Debug, Clone, PartialEq)]
pub struct ExtractionPage {
    page_number: PageNumber,
    width: NonNegativeFinite,
    height: NonNegativeFinite,
    elements: Vec<RawElement>,
}

impl ExtractionPage {
    pub fn new(
        page_number: PageNumber,
        width: f64,
        height: f64,
        elements: Vec<RawElement>,
    ) -> Result<Self, ValidationError> {
        Ok(Self {
            page_number,
            width: NonNegativeFinite::new(width)?,
            height: NonNegativeFinite::new(height)?,
            elements,
        })
    }

    #[must_use]
    pub const fn page_number(&self) -> PageNumber {
        self.page_number
    }

    #[must_use]
    pub const fn width(&self) -> f64 {
        self.width.get()
    }

    #[must_use]
    pub const fn height(&self) -> f64 {
        self.height.get()
    }

    #[must_use]
    pub fn elements(&self) -> &[RawElement] {
        &self.elements
    }

    #[must_use]
    pub fn into_elements(self) -> Vec<RawElement> {
        self.elements
    }
}

impl Serialize for ExtractionPage {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("ExtractionPage", 4)?;
        state.serialize_field("page_number", &self.page_number)?;
        state.serialize_field("width", &self.width)?;
        state.serialize_field("height", &self.height)?;
        state.serialize_field("elements", &self.elements)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for ExtractionPage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            page_number: PageNumber,
            width: f64,
            height: f64,
            elements: Vec<RawElement>,
        }
        let raw = Raw::deserialize(deserializer)?;
        Self::new(raw.page_number, raw.width, raw.height, raw.elements)
            .map_err(serde::de::Error::custom)
    }
}
