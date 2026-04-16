use super::primitives::ValidationError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A finite `f64` constrained to `>= 0.0`.
///
/// Used for dimensional values (page geometry, bbox components, etc.) where
/// `NaN`, infinity, and negative values are semantically invalid but `0.0` is
/// a legitimate edge case (zero-extent page placeholder, empty bbox).
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct NonNegativeFinite(f64);

impl NonNegativeFinite {
    pub fn new(value: f64) -> Result<Self, ValidationError> {
        if !value.is_finite() {
            return Err(ValidationError::NonFiniteCoordinate);
        }
        if value < 0.0 {
            return Err(ValidationError::NegativeDimension);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }

    /// Canonical zero value. Safe because `0.0` is finite and non-negative.
    #[must_use]
    pub const fn zero() -> Self {
        Self(0.0)
    }
}

impl Serialize for NonNegativeFinite {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_f64(self.0)
    }
}

impl<'de> Deserialize<'de> for NonNegativeFinite {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = f64::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// A finite `f64` constrained to `> 0.0`.
///
/// Used for strictly-positive values (font size, glyph advance) where zero is
/// a degenerate state that indicates a producer bug.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct PositiveFinite(f64);

impl PositiveFinite {
    pub fn new(value: f64) -> Result<Self, ValidationError> {
        if !value.is_finite() {
            return Err(ValidationError::NonFiniteCoordinate);
        }
        if value <= 0.0 {
            return Err(ValidationError::NonPositiveValue);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

impl Serialize for PositiveFinite {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_f64(self.0)
    }
}

impl<'de> Deserialize<'de> for PositiveFinite {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = f64::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
#[path = "scalars_tests.rs"]
mod tests;
