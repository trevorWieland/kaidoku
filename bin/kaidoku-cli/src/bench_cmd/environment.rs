use super::BenchEnvironment;
use std::process::Command;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) fn benchmark_environment() -> BenchEnvironment {
    let os = std::env::consts::OS.to_string();
    let arch = std::env::consts::ARCH.to_string();
    let cpu_logical_cores = thread::available_parallelism().map_or(1, usize::from);
    let profile = if cfg!(debug_assertions) {
        "debug".to_string()
    } else {
        "release".to_string()
    };
    let rustc_version = rustc_version();

    BenchEnvironment {
        generated_at_unix_seconds: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map_or(0, |duration| duration.as_secs()),
        runner_class: runner_class(&os, &arch, &profile, cpu_logical_cores, &rustc_version),
        os,
        arch,
        cpu_logical_cores,
        profile,
        rustc_version,
        hostname: hostname(),
        cpu_governor: cpu_governor(),
    }
}

pub(super) fn runner_class(
    os: &str,
    arch: &str,
    profile: &str,
    cpu_logical_cores: usize,
    rustc_version: &str,
) -> String {
    format!(
        "{os}-{arch}-{profile}-{}-rustc{}",
        cpu_core_tier(cpu_logical_cores),
        rustc_track(rustc_version),
    )
}

pub(super) fn cpu_core_tier(cpu_logical_cores: usize) -> &'static str {
    match cpu_logical_cores {
        0..=4 => "core-small",
        5..=8 => "core-medium",
        9..=16 => "core-large",
        _ => "core-xlarge",
    }
}

fn rustc_track(rustc_version: &str) -> String {
    let version = rustc_version.split_whitespace().nth(1).unwrap_or("unknown");
    let mut parts = version.split('.');
    let major = parts.next().unwrap_or("unknown");
    let minor = parts.next().unwrap_or("unknown");
    format!("{major}.{minor}")
}

fn rustc_version() -> String {
    Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map_or_else(|| "unknown".to_string(), |value| value.trim().to_string())
}

fn hostname() -> Option<String> {
    std::env::var("HOSTNAME")
        .ok()
        .or_else(|| std::env::var("COMPUTERNAME").ok())
}

fn cpu_governor() -> Option<String> {
    if std::env::consts::OS != "linux" {
        return None;
    }

    std::fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor")
        .ok()
        .map(|value| value.trim().to_string())
}
