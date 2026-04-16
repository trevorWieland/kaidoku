use super::environment;
use super::{
    BENCH_SCHEMA_VERSION, BenchBaselineStore, BenchReport, REQUIRED_BASELINE_RUNNER_CLASSES,
};
use anyhow::{Context, Result, bail};
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;

/// Write the supplied report to `path`, multiplexing into the baseline store
/// when the file is the canonical baseline and overwriting the single-report
/// current report otherwise.
pub(super) fn write_bench_report(path: &Path, report: &BenchReport) -> Result<()> {
    if is_baseline_path(path) {
        let mut store = if path.exists() {
            load_bench_baseline_store(path).unwrap_or(BenchBaselineStore {
                schema_version: BENCH_SCHEMA_VERSION.to_string(),
                reports: Vec::new(),
            })
        } else {
            BenchBaselineStore {
                schema_version: BENCH_SCHEMA_VERSION.to_string(),
                reports: Vec::new(),
            }
        };

        if store.schema_version != BENCH_SCHEMA_VERSION {
            store = BenchBaselineStore {
                schema_version: BENCH_SCHEMA_VERSION.to_string(),
                reports: Vec::new(),
            };
        }

        if let Some(existing) = store
            .reports
            .iter_mut()
            .find(|candidate| candidate.environment.runner_class == report.environment.runner_class)
        {
            *existing = report.clone();
        } else {
            store.reports.push(report.clone());
            store.reports.sort_by(|left, right| {
                left.environment
                    .runner_class
                    .cmp(&right.environment.runner_class)
            });
        }

        let mut serialized = serde_json::to_string_pretty(&store)?;
        serialized.push('\n');
        fs::write(path, serialized)
            .with_context(|| format!("failed writing benchmark report {}", path.display()))?;
        return Ok(());
    }

    let mut serialized = serde_json::to_string_pretty(report)?;
    serialized.push('\n');
    fs::write(path, serialized)
        .with_context(|| format!("failed writing benchmark report {}", path.display()))?;
    Ok(())
}

pub(super) fn load_bench_baseline_store(path: &Path) -> Result<BenchBaselineStore> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed reading baseline benchmark {}", path.display()))?;

    let mut store = if let Ok(store) = serde_json::from_str::<BenchBaselineStore>(&content) {
        if store.schema_version != BENCH_SCHEMA_VERSION {
            bail!(
                "unexpected benchmark schema {} in {}",
                store.schema_version,
                path.display(),
            );
        }
        store
    } else {
        let report: BenchReport = serde_json::from_str(&content)
            .with_context(|| format!("failed parsing baseline benchmark {}", path.display()))?;
        if report.schema_version != BENCH_SCHEMA_VERSION {
            bail!(
                "unexpected benchmark schema {} in {}",
                report.schema_version,
                path.display(),
            );
        }
        BenchBaselineStore {
            schema_version: BENCH_SCHEMA_VERSION.to_string(),
            reports: vec![report],
        }
    };

    // Backfill empty `compat_class` values so historical baselines lifted from
    // the `v4` schema compare correctly under the new fallback path.
    for report in &mut store.reports {
        if report.environment.compat_class.is_empty() {
            report.environment.compat_class = environment::compat_class(
                &report.environment.os,
                &report.environment.arch,
                &report.environment.profile,
            );
        }
    }

    validate_required_runner_classes(&store, path)?;
    Ok(store)
}

pub(super) fn validate_required_runner_classes(
    store: &BenchBaselineStore,
    path: &Path,
) -> Result<()> {
    let present = store
        .reports
        .iter()
        .map(|report| report.environment.runner_class.as_str())
        .collect::<BTreeSet<_>>();
    let missing = REQUIRED_BASELINE_RUNNER_CLASSES
        .iter()
        .copied()
        .filter(|class| !present.contains(class))
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(());
    }
    bail!(
        "benchmark baseline {} is missing required runner class(es): [{}]. Seed them with \
         `just phase1-bench-refresh` on the corresponding runner.",
        path.display(),
        missing.join(", ")
    );
}

fn is_baseline_path(path: &Path) -> bool {
    path.file_name() == Some(OsStr::new("benchmarks.baseline.json"))
}
