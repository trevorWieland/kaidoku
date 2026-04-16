use super::matrix::Matrix;

const DEFAULT_FONT_SIZE: f64 = 12.0;

#[derive(Debug, Clone)]
pub(super) struct TextState {
    text_matrix: Matrix,
    line_matrix: Matrix,
    leading: f64,
    char_spacing: f64,
    word_spacing: f64,
    h_scaling: f64,
    rise: f64,
    font_key: Option<Vec<u8>>,
    /// Current font size in points.
    ///
    /// Stored as `Option<f64>` so that malformed producer behaviour (e.g.
    /// `Tf /F1 0 Tf`, `NaN`, or negative zero) marks the state as
    /// text-emission-disabled rather than propagating a non-positive value
    /// into `PositiveFinite`, which would abort the whole extraction. Callers
    /// consult [`font_size_valid`] before emitting glyphs.
    font_size: Option<f64>,
}

impl Default for TextState {
    fn default() -> Self {
        Self {
            text_matrix: Matrix::identity(),
            line_matrix: Matrix::identity(),
            leading: 0.0,
            char_spacing: 0.0,
            word_spacing: 0.0,
            h_scaling: 100.0,
            rise: 0.0,
            font_key: None,
            font_size: Some(DEFAULT_FONT_SIZE),
        }
    }
}

impl TextState {
    pub(super) fn begin_text_object(&mut self) {
        self.text_matrix = Matrix::identity();
        self.line_matrix = Matrix::identity();
    }

    pub(super) fn set_font(&mut self, key: Option<Vec<u8>>, size: f64) {
        self.font_key = key;
        // Zero, negative, NaN, and ±∞ font sizes are producer defects rather
        // than fatal content — treat them as "text emission disabled" so a
        // broken Tf operator does not abort the entire document. Emission
        // sites check `font_size_valid()` before touching this field.
        self.font_size = if size.is_finite() && size.abs() > 0.0 {
            Some(size.abs())
        } else {
            None
        };
    }

    pub(super) fn set_char_spacing(&mut self, char_spacing: f64) {
        self.char_spacing = char_spacing;
    }

    pub(super) fn set_word_spacing(&mut self, word_spacing: f64) {
        self.word_spacing = word_spacing;
    }

    pub(super) fn set_horizontal_scaling(&mut self, h_scaling: f64) {
        self.h_scaling = h_scaling;
    }

    pub(super) fn set_leading(&mut self, leading: f64) {
        self.leading = leading;
    }

    pub(super) fn set_text_rise(&mut self, rise: f64) {
        self.rise = rise;
    }

    pub(super) fn move_text_position(&mut self, tx: f64, ty: f64, set_leading: bool) {
        let translation = Matrix::translation(tx, ty);
        self.line_matrix = self.line_matrix.concatenate(translation);
        self.text_matrix = self.line_matrix;
        if set_leading {
            self.leading = -ty;
        }
    }

    pub(super) fn set_text_matrix(&mut self, matrix: Matrix) {
        self.text_matrix = matrix;
        self.line_matrix = matrix;
    }

    pub(super) fn next_line(&mut self) {
        self.move_text_position(0.0, -self.leading, false);
    }

    pub(super) fn advance_text(&mut self, tx: f64) {
        self.text_matrix = self.text_matrix.concatenate(Matrix::translation(tx, 0.0));
    }

    pub(super) fn apply_tj_adjustment(&mut self, adjustment: f64) {
        let tx = -(adjustment / 1000.0) * self.font_size() * self.horizontal_scale_factor();
        self.advance_text(tx);
    }

    #[must_use]
    pub(super) fn glyph_transform(&self, ctm: Matrix, glyph_width: f64) -> Matrix {
        let text_space = self
            .text_matrix
            .concatenate(Matrix::translation(0.0, self.rise));
        ctm.concatenate(text_space).concatenate(Matrix::scale(
            glyph_width.max(f64::EPSILON),
            self.font_size().max(f64::EPSILON),
        ))
    }

    #[must_use]
    pub(super) fn current_origin(&self, ctm: Matrix) -> (f64, f64) {
        let text_space = self
            .text_matrix
            .concatenate(Matrix::translation(0.0, self.rise));
        ctm.concatenate(text_space).transform(0.0, 0.0)
    }

    #[must_use]
    pub(super) fn font_size(&self) -> f64 {
        self.font_size.unwrap_or(DEFAULT_FONT_SIZE)
    }

    #[must_use]
    pub(super) fn font_size_valid(&self) -> bool {
        self.font_size.is_some()
    }

    #[must_use]
    pub(super) fn font_key(&self) -> Option<&[u8]> {
        self.font_key.as_deref()
    }

    #[must_use]
    pub(super) fn char_spacing(&self) -> f64 {
        self.char_spacing
    }

    #[must_use]
    pub(super) fn word_spacing(&self) -> f64 {
        self.word_spacing
    }

    #[must_use]
    pub(super) fn horizontal_scale_factor(&self) -> f64 {
        self.h_scaling / 100.0
    }
}

#[derive(Debug, Clone)]
pub(super) struct GraphicsState {
    ctm: Matrix,
    stack: Vec<Matrix>,
}

impl Default for GraphicsState {
    fn default() -> Self {
        Self {
            ctm: Matrix::identity(),
            stack: Vec::new(),
        }
    }
}

impl GraphicsState {
    #[must_use]
    pub(super) fn with_ctm(ctm: Matrix) -> Self {
        Self {
            ctm,
            stack: Vec::new(),
        }
    }

    pub(super) fn concatenate_ctm(&mut self, matrix: Matrix) {
        self.ctm = self.ctm.concatenate(matrix);
    }

    pub(super) fn save(&mut self) {
        self.stack.push(self.ctm);
    }

    pub(super) fn restore(&mut self) {
        if let Some(previous) = self.stack.pop() {
            self.ctm = previous;
        }
    }

    #[must_use]
    pub(super) fn ctm(&self) -> Matrix {
        self.ctm
    }
}
