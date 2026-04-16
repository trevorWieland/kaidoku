//! CLI `--pages` argument grammar.
//!
//! The parser is intentionally defensive: an adversarial input like
//! `--pages 1-4000000000,2` must never materialise billions of page numbers
//! before the core bounds check. The lexer therefore tokenises once, computes
//! cardinality against a hard cap, and only allocates the final `Vec<u32>`
//! (or `PageRange`) after the cap check passes.

use anyhow::{Context, Result, bail};
use kaidoku_core::{PageRange, PageSelection, default_max_pages};

/// Upper bound on the number of pages the CLI will materialise from a
/// `--pages` specification. Matches [`default_max_pages`] so the CLI never
/// manufactures a selection the core would reject anyway.
pub(super) const DEFAULT_PAGE_SELECTION_CAP: u64 = default_max_pages() as u64;

/// Parse a `--pages` specification into a [`PageSelection`].
///
/// Grammar (whitespace around each atom is tolerated):
/// ```text
/// spec       := "all" / tokens
/// tokens     := token ("," token)*
/// token      := single / range
/// single     := u32
/// range      := u32 "-" u32
/// ```
///
/// Malformed trailing/leading separators (`"1,"`, `",1"`, `"1,,2"`, `"-5"`,
/// `"5-"`) are rejected with a specific diagnostic instead of silently
/// dropping empty segments. Cardinality is bounded by
/// [`DEFAULT_PAGE_SELECTION_CAP`] so pathological ranges fail fast.
pub(super) fn parse_pages_spec(spec: Option<&str>) -> Result<PageSelection> {
    let Some(raw_spec) = spec else {
        return Ok(PageSelection::All);
    };

    let trimmed = raw_spec.trim();
    if trimmed.eq_ignore_ascii_case("all") {
        return Ok(PageSelection::All);
    }
    if trimmed.is_empty() {
        bail!("--pages cannot be empty; use `all` or omit the flag");
    }

    let tokens = lex_tokens(trimmed)?;
    let mut total_cardinality: u64 = 0;
    for token in &tokens {
        let cardinality = u64::from(token.cardinality());
        total_cardinality = total_cardinality
            .checked_add(cardinality)
            .with_context(|| {
                format!(
                    "--pages cumulative cardinality overflow at token `{}`",
                    token.source
                )
            })?;
        if total_cardinality > DEFAULT_PAGE_SELECTION_CAP {
            bail!(
                "--pages cardinality {total_cardinality} exceeds cap {DEFAULT_PAGE_SELECTION_CAP} \
                 (cap matches max_pages); use a narrower range or raise the core max_pages option"
            );
        }
    }

    if tokens.len() == 1
        && let Token {
            kind: TokenKind::Range(range),
            ..
        } = &tokens[0]
    {
        return Ok(PageSelection::Range(*range));
    }

    let mut pages = Vec::with_capacity(usize::try_from(total_cardinality).with_context(|| {
        format!("--pages cardinality {total_cardinality} does not fit in usize")
    })?);
    for token in tokens {
        match token.kind {
            TokenKind::Single(page) => pages.push(page),
            TokenKind::Range(range) => {
                for page in range.start()..=range.end() {
                    pages.push(page);
                }
            }
        }
    }

    PageSelection::from_pages(pages).context("building --pages selection")
}

#[derive(Debug, Clone)]
struct Token {
    kind: TokenKind,
    source: String,
}

#[derive(Debug, Clone, Copy)]
enum TokenKind {
    Single(u32),
    Range(PageRange),
}

impl Token {
    fn cardinality(&self) -> u32 {
        match self.kind {
            TokenKind::Single(_) => 1,
            TokenKind::Range(range) => range.len(),
        }
    }
}

fn lex_tokens(spec: &str) -> Result<Vec<Token>> {
    let mut tokens = Vec::new();
    for raw in spec.split(',') {
        let entry = raw.trim();
        if entry.is_empty() {
            bail!(
                "--pages contains an empty segment (leading/trailing/duplicate comma in `{spec}`)"
            );
        }

        if let Some((start_raw, end_raw)) = entry.split_once('-') {
            let start_trimmed = start_raw.trim();
            let end_trimmed = end_raw.trim();
            if start_trimmed.is_empty() || end_trimmed.is_empty() {
                bail!("--pages range `{entry}` is missing a bound");
            }
            let start = parse_u32(start_trimmed, entry)?;
            let end = parse_u32(end_trimmed, entry)?;
            let range =
                PageRange::new(start, end).with_context(|| format!("invalid range `{entry}`"))?;
            tokens.push(Token {
                kind: TokenKind::Range(range),
                source: entry.to_string(),
            });
        } else {
            let page = parse_u32(entry, entry)?;
            if page == 0 {
                bail!("--pages entry `{entry}` must be >= 1");
            }
            tokens.push(Token {
                kind: TokenKind::Single(page),
                source: entry.to_string(),
            });
        }
    }
    if tokens.is_empty() {
        bail!("--pages produced an empty selection");
    }
    Ok(tokens)
}

fn parse_u32(raw: &str, entry: &str) -> Result<u32> {
    raw.parse::<u32>()
        .with_context(|| format!("invalid page number in `{entry}`"))
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_PAGE_SELECTION_CAP, parse_pages_spec};
    use kaidoku_core::PageSelection;

    #[test]
    fn parses_single_page() {
        let selection = parse_pages_spec(Some("4")).expect("parse");
        assert!(matches!(selection, PageSelection::Explicit(_)));
    }

    #[test]
    fn parses_contiguous_range_as_range() {
        let selection = parse_pages_spec(Some("2-5")).expect("parse");
        assert!(matches!(selection, PageSelection::Range(_)));
    }

    #[test]
    fn parses_mixed_spec_as_explicit() {
        let selection = parse_pages_spec(Some("1-3,5,7-9")).expect("parse");
        assert_eq!(
            selection.explicit_pages(),
            Some(&[1_u32, 2, 3, 5, 7, 8, 9][..])
        );
    }

    #[test]
    fn rejects_trailing_comma() {
        let err = parse_pages_spec(Some("1-3,")).expect_err("trailing comma");
        assert!(err.to_string().contains("empty segment"));
    }

    #[test]
    fn rejects_leading_comma() {
        let err = parse_pages_spec(Some(",1")).expect_err("leading comma");
        assert!(err.to_string().contains("empty segment"));
    }

    #[test]
    fn rejects_zero_page() {
        let err = parse_pages_spec(Some("0")).expect_err("zero page");
        assert!(err.to_string().contains(">= 1"));
    }

    #[test]
    fn rejects_reverse_range() {
        let err = parse_pages_spec(Some("5-2")).expect_err("reverse range");
        assert!(err.to_string().contains("invalid range"));
    }

    #[test]
    fn rejects_cardinality_overflow_before_materializing() {
        let err = parse_pages_spec(Some("1-4000000000,2")).expect_err("cap");
        assert!(err.to_string().contains("cardinality"));
    }

    #[test]
    fn accepts_cap_boundary() {
        let spec = format!("1-{DEFAULT_PAGE_SELECTION_CAP}");
        let selection = parse_pages_spec(Some(&spec)).expect("cap boundary");
        assert!(matches!(selection, PageSelection::Range(_)));
    }

    #[test]
    fn rejects_cap_plus_one() {
        let spec = format!("1-{}", DEFAULT_PAGE_SELECTION_CAP + 1);
        let err = parse_pages_spec(Some(&spec)).expect_err("cap exceeded");
        assert!(err.to_string().contains("cardinality"));
    }

    #[test]
    fn accepts_all_keyword() {
        let selection = parse_pages_spec(Some("all")).expect("all");
        assert!(matches!(selection, PageSelection::All));
    }

    #[test]
    fn rejects_empty_input() {
        let err = parse_pages_spec(Some("")).expect_err("empty");
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn rejects_missing_bound_in_range() {
        assert!(parse_pages_spec(Some("-5")).is_err());
        assert!(parse_pages_spec(Some("5-")).is_err());
    }

    #[test]
    fn whitespace_tolerated_inside_token() {
        let selection = parse_pages_spec(Some(" 1 - 3 , 5 ")).expect("whitespace");
        assert_eq!(selection.explicit_pages(), Some(&[1_u32, 2, 3, 5][..]));
    }
}
