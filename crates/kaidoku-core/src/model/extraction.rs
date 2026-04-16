use super::document::{ExtractionPage, FontDescriptor, RawElement};
use super::primitives::{BackendIdentifier, SchemaIdentifier, Sha256Digest, ValidationError};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeSet;

/// Immutable provenance block describing a single extraction run.
///
/// Construction is sealed behind [`ExtractionSource::new`] so the struct
/// cannot be assembled in a half-filled state by downstream callers. All
/// fields are already validated-by-type (`BackendIdentifier`, `Sha256Digest`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractionSource {
    backend: BackendIdentifier,
    input_sha256: Sha256Digest,
    input_bytes: usize,
}

impl ExtractionSource {
    #[must_use]
    pub const fn new(
        backend: BackendIdentifier,
        input_sha256: Sha256Digest,
        input_bytes: usize,
    ) -> Self {
        Self {
            backend,
            input_sha256,
            input_bytes,
        }
    }

    #[must_use]
    pub const fn backend(&self) -> BackendIdentifier {
        self.backend
    }

    #[must_use]
    pub const fn input_sha256(&self) -> &Sha256Digest {
        &self.input_sha256
    }

    #[must_use]
    pub const fn input_bytes(&self) -> usize {
        self.input_bytes
    }
}

impl Serialize for ExtractionSource {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("ExtractionSource", 3)?;
        state.serialize_field("backend", &self.backend)?;
        state.serialize_field("input_sha256", &self.input_sha256)?;
        state.serialize_field("input_bytes", &self.input_bytes)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for ExtractionSource {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            backend: BackendIdentifier,
            input_sha256: Sha256Digest,
            input_bytes: usize,
        }
        let raw = Raw::deserialize(deserializer)?;
        Ok(Self::new(raw.backend, raw.input_sha256, raw.input_bytes))
    }
}

/// Top-level extraction result.
///
/// Construction is sealed behind [`ExtractionDocument::new`], which enforces
/// the canonical document invariants so downstream consumers can rely on
/// them without re-checking:
///
/// - `pages` is non-empty.
/// - `pages` is sorted by ascending `page_number` with no duplicates.
/// - `fonts` IDs are unique.
/// - Every `font_id` referenced by a char/span payload is present in
///   `fonts` (no dangling references).
///
/// Deserialization goes through `new` so JSON that bypasses the constructor
/// still validates invariants before it becomes an `ExtractionDocument`.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtractionDocument {
    schema_version: SchemaIdentifier,
    source: ExtractionSource,
    fonts: Vec<FontDescriptor>,
    pages: Vec<ExtractionPage>,
}

impl ExtractionDocument {
    pub fn new(
        schema_version: SchemaIdentifier,
        source: ExtractionSource,
        fonts: Vec<FontDescriptor>,
        pages: Vec<ExtractionPage>,
    ) -> Result<Self, ValidationError> {
        if pages.is_empty() {
            return Err(ValidationError::EmptyPages);
        }

        let mut last_seen: Option<u32> = None;
        for page in &pages {
            let current = page.page_number().get();
            if let Some(previous) = last_seen
                && current <= previous
            {
                return Err(ValidationError::UnsortedOrDuplicatePage {
                    page_number: current,
                });
            }
            last_seen = Some(current);
        }

        let mut known_ids = BTreeSet::new();
        for descriptor in &fonts {
            if !known_ids.insert(descriptor.id) {
                return Err(ValidationError::DuplicateFontId {
                    font_id: descriptor.id.get(),
                });
            }
        }

        for page in &pages {
            for element in page.elements() {
                let referenced = match element {
                    RawElement::Char { payload, .. } => payload.font_id(),
                    RawElement::Span { payload, .. } => payload.font_id(),
                    RawElement::Image { .. } => None,
                };
                if let Some(font_id) = referenced
                    && !known_ids.contains(&font_id)
                {
                    return Err(ValidationError::UnknownFontReference {
                        font_id: font_id.get(),
                        page_number: page.page_number().get(),
                    });
                }
            }
        }

        Ok(Self {
            schema_version,
            source,
            fonts,
            pages,
        })
    }

    #[must_use]
    pub const fn schema_version(&self) -> SchemaIdentifier {
        self.schema_version
    }

    #[must_use]
    pub const fn source(&self) -> &ExtractionSource {
        &self.source
    }

    #[must_use]
    pub fn fonts(&self) -> &[FontDescriptor] {
        &self.fonts
    }

    #[must_use]
    pub fn pages(&self) -> &[ExtractionPage] {
        &self.pages
    }
}

impl Serialize for ExtractionDocument {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("ExtractionDocument", 4)?;
        state.serialize_field("schema_version", &self.schema_version)?;
        state.serialize_field("source", &self.source)?;
        state.serialize_field("fonts", &self.fonts)?;
        state.serialize_field("pages", &self.pages)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for ExtractionDocument {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            schema_version: SchemaIdentifier,
            source: ExtractionSource,
            fonts: Vec<FontDescriptor>,
            pages: Vec<ExtractionPage>,
        }
        let raw = Raw::deserialize(deserializer)?;
        Self::new(raw.schema_version, raw.source, raw.fonts, raw.pages)
            .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests;
