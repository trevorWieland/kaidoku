use super::{NonNegativeFinite, PositiveFinite};
use crate::ValidationError;
use serde_json::{from_str, to_string};

#[test]
fn non_negative_finite_rejects_nan_infinity_and_negative() {
    assert_eq!(
        NonNegativeFinite::new(f64::NAN),
        Err(ValidationError::NonFiniteCoordinate)
    );
    assert_eq!(
        NonNegativeFinite::new(f64::INFINITY),
        Err(ValidationError::NonFiniteCoordinate)
    );
    assert_eq!(
        NonNegativeFinite::new(f64::NEG_INFINITY),
        Err(ValidationError::NonFiniteCoordinate)
    );
    assert_eq!(
        NonNegativeFinite::new(-1.0),
        Err(ValidationError::NegativeDimension)
    );
}

#[test]
fn non_negative_finite_accepts_zero_and_positive() {
    assert!(NonNegativeFinite::new(0.0).is_ok());
    assert!(NonNegativeFinite::new(1.5).is_ok());
    assert!((NonNegativeFinite::zero().get() - 0.0).abs() < f64::EPSILON);
}

#[test]
fn non_negative_finite_round_trips_json() {
    let value = NonNegativeFinite::new(42.5).expect("value");
    let json = to_string(&value).expect("serialize");
    assert_eq!(json, "42.5");
    let parsed: NonNegativeFinite = from_str(&json).expect("deserialize");
    assert!((parsed.get() - 42.5).abs() < f64::EPSILON);
}

#[test]
fn non_negative_finite_rejects_invalid_json() {
    let err: Result<NonNegativeFinite, _> = from_str("-1.0");
    assert!(err.is_err());
}

#[test]
fn positive_finite_rejects_zero_nan_and_negative() {
    assert_eq!(
        PositiveFinite::new(0.0),
        Err(ValidationError::NonPositiveValue)
    );
    assert_eq!(
        PositiveFinite::new(-0.0),
        Err(ValidationError::NonPositiveValue)
    );
    assert_eq!(
        PositiveFinite::new(-1.0),
        Err(ValidationError::NonPositiveValue)
    );
    assert_eq!(
        PositiveFinite::new(f64::NAN),
        Err(ValidationError::NonFiniteCoordinate)
    );
}

#[test]
fn positive_finite_accepts_positive() {
    assert!(PositiveFinite::new(0.001).is_ok());
    assert!(PositiveFinite::new(12.0).is_ok());
}

#[test]
fn positive_finite_round_trips_json() {
    let value = PositiveFinite::new(12.0).expect("value");
    let json = to_string(&value).expect("serialize");
    assert_eq!(json, "12.0");
    let parsed: PositiveFinite = from_str(&json).expect("deserialize");
    assert!((parsed.get() - 12.0).abs() < f64::EPSILON);
}

#[test]
fn positive_finite_rejects_invalid_json() {
    let err: Result<PositiveFinite, _> = from_str("0.0");
    assert!(err.is_err());
}
