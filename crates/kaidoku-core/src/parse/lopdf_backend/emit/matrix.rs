use crate::ExtractError;
use lopdf::Object;

#[derive(Debug, Clone, Copy)]
pub(super) struct Matrix {
    pub(super) a: f64,
    pub(super) b: f64,
    pub(super) c: f64,
    pub(super) d: f64,
    pub(super) e: f64,
    pub(super) f: f64,
}

impl Matrix {
    pub(super) const fn identity() -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        }
    }

    pub(super) const fn translation(tx: f64, ty: f64) -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: tx,
            f: ty,
        }
    }

    pub(super) const fn scale(sx: f64, sy: f64) -> Self {
        Self {
            a: sx,
            b: 0.0,
            c: 0.0,
            d: sy,
            e: 0.0,
            f: 0.0,
        }
    }

    pub(super) fn from_operands(operands: &[Object]) -> Result<Self, ExtractError> {
        if operands.len() != 6 {
            return Err(ExtractError::InvariantViolation {
                reason: "matrix operator requires 6 operands".to_string(),
            });
        }
        Ok(Self {
            a: f64::from(object_to_f32(&operands[0])?),
            b: f64::from(object_to_f32(&operands[1])?),
            c: f64::from(object_to_f32(&operands[2])?),
            d: f64::from(object_to_f32(&operands[3])?),
            e: f64::from(object_to_f32(&operands[4])?),
            f: f64::from(object_to_f32(&operands[5])?),
        })
    }

    #[must_use]
    pub(super) const fn from_values(values: [f64; 6]) -> Self {
        Self {
            a: values[0],
            b: values[1],
            c: values[2],
            d: values[3],
            e: values[4],
            f: values[5],
        }
    }

    #[must_use]
    pub(super) fn concatenate(self, next: Self) -> Self {
        Self {
            a: (self.a * next.a) + (self.b * next.c),
            b: (self.a * next.b) + (self.b * next.d),
            c: (self.c * next.a) + (self.d * next.c),
            d: (self.c * next.b) + (self.d * next.d),
            e: (self.e * next.a) + (self.f * next.c) + next.e,
            f: (self.e * next.b) + (self.f * next.d) + next.f,
        }
    }

    #[must_use]
    pub(super) fn to_bbox(self) -> Option<(f64, f64, f64, f64)> {
        let p0 = self.transform(0.0, 0.0);
        let p1 = self.transform(1.0, 0.0);
        let p2 = self.transform(0.0, 1.0);
        let p3 = self.transform(1.0, 1.0);

        let min_x = p0.0.min(p1.0).min(p2.0).min(p3.0);
        let max_x = p0.0.max(p1.0).max(p2.0).max(p3.0);
        let min_y = p0.1.min(p1.1).min(p2.1).min(p3.1);
        let max_y = p0.1.max(p1.1).max(p2.1).max(p3.1);

        let width = (max_x - min_x).abs();
        let height = (max_y - min_y).abs();
        if width == 0.0 || height == 0.0 {
            return None;
        }
        Some((min_x, min_y, width, height))
    }

    #[must_use]
    pub(super) fn transform(self, x: f64, y: f64) -> (f64, f64) {
        (
            (self.a * x) + (self.c * y) + self.e,
            (self.b * x) + (self.d * y) + self.f,
        )
    }
}

fn object_to_f32(value: &Object) -> Result<f32, ExtractError> {
    value
        .as_float()
        .map_err(|error| ExtractError::ContentDecode {
            reason: error.to_string(),
        })
}
