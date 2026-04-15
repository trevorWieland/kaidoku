use crate::ExtractCommand;
use anyhow::{Context, Result, anyhow, bail};
use kaidoku_core::{ExtractOptions, PageRange, PageSelection, extract_pdf, to_canonical_json};
use rayon::ThreadPoolBuilder;
use rayon::prelude::*;
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

pub(super) fn run_extract(command: ExtractCommand) -> Result<()> {
    if command.jobs == 0 {
        bail!("--jobs must be >= 1");
    }

    let selection = parse_pages_spec(command.pages.as_deref())?;
    fs::create_dir_all(&command.output).with_context(|| {
        format!(
            "failed creating output directory {}",
            command.output.display()
        )
    })?;

    let mut inputs = command.input;
    inputs.sort();

    let planned_outputs = plan_output_paths(&inputs, &command.output)?;

    let pool = ThreadPoolBuilder::new()
        .num_threads(command.jobs)
        .build()
        .context("failed to build extraction thread pool")?;

    pool.install(|| {
        planned_outputs
            .par_iter()
            .map(|(input_path, output_path)| {
                extract_one(input_path, output_path, selection.clone())
            })
            .collect::<Result<Vec<_>>>()
    })?;

    Ok(())
}

fn extract_one(input_path: &Path, output_path: &Path, page_selection: PageSelection) -> Result<()> {
    let bytes = fs::read(input_path)
        .with_context(|| format!("failed reading input file {}", input_path.display()))?;

    let options = ExtractOptions {
        page_selection,
        ..ExtractOptions::default()
    };

    let document = extract_pdf(&bytes, options)
        .with_context(|| format!("failed extracting {}", input_path.display()))?;

    let json = to_canonical_json(&document)?;
    fs::write(output_path, json)
        .with_context(|| format!("failed writing output {}", output_path.display()))?;

    Ok(())
}

fn plan_output_paths(inputs: &[PathBuf], output_dir: &Path) -> Result<Vec<(PathBuf, PathBuf)>> {
    let mut groups: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for input in inputs {
        let stem = extraction_output_stem(input)?;
        groups.entry(stem).or_default().push(input.clone());
    }

    let mut planned = Vec::with_capacity(inputs.len());
    for (stem, mut grouped_inputs) in groups {
        grouped_inputs.sort_by_key(|path| stable_input_hint(path));

        if grouped_inputs.len() == 1 {
            let input = grouped_inputs.pop().expect("single input in group");
            planned.push((input, output_dir.join(format!("{stem}.json"))));
            continue;
        }

        for (index, input) in grouped_inputs.into_iter().enumerate() {
            let suffix = u32::try_from(index)
                .map_err(|_| anyhow!("duplicate index overflow for output stem {stem}"))?
                .saturating_add(1);
            planned.push((input, output_dir.join(format!("{stem}_{suffix:02}.json"))));
        }
    }

    planned.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(planned)
}

fn extraction_output_stem(input_path: &Path) -> Result<String> {
    let stem = input_path
        .file_stem()
        .and_then(OsStr::to_str)
        .ok_or_else(|| anyhow!("invalid input filename: {}", input_path.display()))?;
    Ok(sanitize_component(stem))
}

fn sanitize_component(input: &str) -> String {
    let sanitized = input
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();

    if sanitized.is_empty() {
        "unnamed".to_string()
    } else {
        sanitized
    }
}

fn stable_input_hint(input_path: &Path) -> String {
    let parent = input_path
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(OsStr::to_str)
        .unwrap_or("root");
    let file_name = input_path
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("unnamed");
    format!(
        "{}:{}",
        sanitize_component(parent),
        sanitize_component(file_name)
    )
}

fn parse_pages_spec(spec: Option<&str>) -> Result<PageSelection> {
    let Some(raw_spec) = spec else {
        return Ok(PageSelection::All);
    };

    let trimmed = raw_spec.trim();
    if trimmed.eq_ignore_ascii_case("all") {
        return Ok(PageSelection::All);
    }

    let mut explicit_pages = Vec::new();
    let mut parsed_range: Option<PageRange> = None;

    for token in trimmed.split(',') {
        let entry = token.trim();
        if entry.is_empty() {
            continue;
        }

        if let Some((start, end)) = entry.split_once('-') {
            let start_page: u32 = start
                .trim()
                .parse()
                .with_context(|| format!("invalid page in range: {entry}"))?;
            let end_page: u32 = end
                .trim()
                .parse()
                .with_context(|| format!("invalid page in range: {entry}"))?;
            let range = PageRange::new(start_page, end_page)?;
            if trimmed.split(',').count() == 1 {
                parsed_range = Some(range);
            } else {
                for page in range.start()..=range.end() {
                    explicit_pages.push(page);
                }
            }
        } else {
            let page: u32 = entry
                .parse()
                .with_context(|| format!("invalid page entry: {entry}"))?;
            explicit_pages.push(page);
        }
    }

    if let Some(range) = parsed_range {
        return Ok(PageSelection::Range(range));
    }

    if explicit_pages.is_empty() {
        bail!("--pages produced an empty selection");
    }

    Ok(PageSelection::from_pages(explicit_pages)?)
}

#[cfg(test)]
mod tests {
    use super::{PageSelection, extraction_output_stem, parse_pages_spec, plan_output_paths};
    use std::path::Path;

    #[test]
    fn pages_spec_all_defaults() {
        let selection = parse_pages_spec(None);
        assert!(selection.is_ok());

        let Ok(selection) = selection else { return };
        assert!(matches!(selection, PageSelection::All));
    }

    #[test]
    fn pages_spec_range_parses() {
        let selection = parse_pages_spec(Some("2-5"));
        assert!(selection.is_ok());

        let Ok(selection) = selection else { return };
        assert!(matches!(selection, PageSelection::Range(_)));
    }

    #[test]
    fn pages_spec_list_parses_to_explicit() {
        let selection = parse_pages_spec(Some("3,1,3,2"));
        assert!(selection.is_ok());

        let Ok(selection) = selection else { return };
        assert!(matches!(selection, PageSelection::Explicit(_)));
    }

    #[test]
    fn output_stems_are_stable_across_clone_paths() {
        let output_a = extraction_output_stem(Path::new("/tmp/alpha/input.pdf"));
        let output_b = extraction_output_stem(Path::new("/different/root/input.pdf"));

        assert!(output_a.is_ok());
        assert!(output_b.is_ok());
        let (Ok(output_a), Ok(output_b)) = (output_a, output_b) else {
            return;
        };
        assert_eq!(output_a, output_b);
    }

    #[test]
    fn preflight_duplicate_detection_passes_unique_outputs() {
        let planned = plan_output_paths(
            &[
                Path::new("/tmp/alpha/input.pdf").to_path_buf(),
                Path::new("/tmp/beta/input.pdf").to_path_buf(),
            ],
            Path::new("/tmp/out"),
        );

        assert!(planned.is_ok());
    }
}
