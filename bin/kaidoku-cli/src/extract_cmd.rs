mod pages;

use crate::ExtractCommand;
use anyhow::{Context, Result, anyhow, bail};
use kaidoku_core::{
    ExtractOptions, PageSelection, default_max_input_bytes, extract_pdf, to_canonical_json,
};
use pages::parse_pages_spec;
use rayon::ThreadPoolBuilder;
use rayon::prelude::*;
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

pub(super) fn run_extract(command: ExtractCommand) -> Result<()> {
    if command.jobs == 0 {
        bail!("--jobs must be >= 1");
    }

    let selection = parse_pages_spec(command.pages.as_deref())?;
    let max_wall_time_ms = command.max_wall_time_ms;
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
                extract_one(input_path, output_path, selection.clone(), max_wall_time_ms)
            })
            .collect::<Result<Vec<_>>>()
    })?;

    Ok(())
}

fn extract_one(
    input_path: &Path,
    output_path: &Path,
    page_selection: PageSelection,
    max_wall_time_ms: u64,
) -> Result<()> {
    let bytes = read_input_with_limit(input_path, default_max_input_bytes())?;

    let options = ExtractOptions::builder()
        .page_selection(page_selection)
        .max_wall_time_ms(max_wall_time_ms)
        .build()
        .context("invalid extraction options")?;

    let document = extract_pdf(&bytes, options)
        .with_context(|| format!("failed extracting {}", input_path.display()))?;

    let json = to_canonical_json(&document)?;
    fs::write(output_path, json)
        .with_context(|| format!("failed writing output {}", output_path.display()))?;

    Ok(())
}

/// Read `path` fully into memory, but never buffer more than `limit` bytes.
///
/// The check is enforced twice: first via `fs::metadata` so oversized files
/// are rejected before any allocation, and second via a bounded
/// `Read::take(limit + 1)` so a TOCTOU grow between `metadata()` and `open()`
/// still fails loudly instead of allocating an unbounded buffer.
fn read_input_with_limit(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("failed stat-ing input file {}", path.display()))?;
    if !metadata.is_file() {
        bail!("input path {} is not a regular file", path.display());
    }

    let declared_size = usize::try_from(metadata.len())
        .with_context(|| format!("file size does not fit into usize for {}", path.display()))?;
    if declared_size > limit {
        bail!(
            "input file {} is {} bytes, which exceeds the configured size limit of {} bytes",
            path.display(),
            declared_size,
            limit,
        );
    }

    let file = fs::File::open(path)
        .with_context(|| format!("failed reading input file {}", path.display()))?;
    let take_limit = u64::try_from(limit)
        .ok()
        .and_then(|value| value.checked_add(1))
        .context("size limit overflows u64")?;
    let mut buffer = Vec::with_capacity(declared_size);
    file.take(take_limit)
        .read_to_end(&mut buffer)
        .with_context(|| format!("failed reading input file {}", path.display()))?;

    if buffer.len() > limit {
        bail!(
            "input file {} grew past the {}-byte limit while being read",
            path.display(),
            limit,
        );
    }

    Ok(buffer)
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

#[cfg(test)]
mod tests {
    use super::{extraction_output_stem, plan_output_paths, read_input_with_limit};
    use std::io::Write;
    use std::path::Path;

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
    fn read_input_with_limit_rejects_oversize_via_metadata() {
        let tmp = std::env::temp_dir().join("kaidoku_extract_cmd_oversize.bin");
        {
            let mut file = std::fs::File::create(&tmp).expect("create tmp file");
            file.write_all(&[0_u8; 128]).expect("write tmp file");
        }
        let result = read_input_with_limit(&tmp, 16);
        let _ = std::fs::remove_file(&tmp);
        let err = result.expect_err("metadata preflight should reject oversize file");
        assert!(
            err.to_string()
                .contains("exceeds the configured size limit")
        );
    }

    #[test]
    fn read_input_with_limit_accepts_small_file() {
        let tmp = std::env::temp_dir().join("kaidoku_extract_cmd_small.bin");
        {
            let mut file = std::fs::File::create(&tmp).expect("create tmp file");
            file.write_all(b"0123456789").expect("write tmp file");
        }
        let result = read_input_with_limit(&tmp, 1024);
        let _ = std::fs::remove_file(&tmp);
        let bytes = result.expect("small file should read within limit");
        assert_eq!(bytes, b"0123456789");
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
