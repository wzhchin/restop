use anyhow::{bail, Context, Result};
use chin_tools::AResult;
use glob::glob;
use once_cell::sync::Lazy;
use regex::Regex;
use std::path::{Path, PathBuf};

const KNOWN_HWMONS: &[&str] = &["zenpower", "coretemp", "k10temp"];

const KNOWN_THERMAL_ZONES: &[&str] = &["x86_pkg_temp", "acpitz"];

static RE_LSCPU_MODEL_NAME: Lazy<Regex> = Lazy::new(|| Regex::new(r"Model name:\s*(.*)").unwrap());

static RE_LSCPU_ARCHITECTURE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"Architecture:\s*(.*)").unwrap());

static RE_LSCPU_CPUS: Lazy<Regex> = Lazy::new(|| Regex::new(r"CPU\(s\):\s*(.*)").unwrap());

static RE_LSCPU_SOCKETS: Lazy<Regex> = Lazy::new(|| Regex::new(r"Socket\(s\):\s*(.*)").unwrap());

static RE_LSCPU_CORES: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"Core\(s\) per socket:\s*(.*)").unwrap());

static RE_LSCPU_VIRTUALIZATION: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"Virtualization:\s*(.*)").unwrap());

static RE_LSCPU_MAX_MHZ: Lazy<Regex> = Lazy::new(|| Regex::new(r"CPU max MHz:\s*(.*)").unwrap());

static CPU_TEMPERATURE_PATH: Lazy<Option<PathBuf>> = Lazy::new(|| {
    let cpu_temperature_path =
        search_for_hwmons(KNOWN_HWMONS).or_else(|| search_for_thermal_zones(KNOWN_THERMAL_ZONES));

    if let Some((sensor, path)) = &cpu_temperature_path {
        log::debug!(
            "CPU temperature sensor located at {} ({sensor})",
            path.display()
        );
    } else {
        log::warn!("No sensor for CPU temperature found!");
    }

    cpu_temperature_path.map(|(_, path)| path)
});

/// Looks for hwmons with the given names.
/// This function is a bit inefficient since the `names` array is considered to be ordered by priority.
fn search_for_hwmons(names: &[&'static str]) -> Option<(&'static str, PathBuf)> {
    for temp_name in names {
        for path in (glob("/sys/class/hwmon/hwmon*").unwrap()).flatten() {
            if let Ok(read_name) = std::fs::read_to_string(path.join("name")) {
                if &read_name.trim_end() == temp_name {
                    return Some((temp_name, path.join("temp1_input")));
                }
            }
        }
    }

    None
}

/// Looks for thermal zones with the given types.
/// This function is a bit inefficient since the `types` array is considered to be ordered by priority.
fn search_for_thermal_zones(types: &[&'static str]) -> Option<(&'static str, PathBuf)> {
    for temp_type in types {
        for path in (glob("/sys/class/thermal/thermal_zone*").unwrap()).flatten() {
            if let Ok(read_type) = std::fs::read_to_string(path.join("type")) {
                if &read_type.trim_end() == temp_type {
                    return Some((temp_type, path.join("temp")));
                }
            }
        }
    }

    None
}

#[derive(Clone, Debug)]
pub struct CpuData {
    pub new_total_usage: (u64, u64),
    pub new_thread_usages: Vec<(u64, u64)>,
    pub temperature: Option<f32>,
    pub frequency: Option<u64>,
}

impl CpuData {
    pub fn fetch(logical_cpus: usize) -> AResult<Self> {
        // Single /proc/stat read for all cores (was N+1 full-file reads).
        let (new_total_usage, new_thread_usages) = read_all_cpu_usages(logical_cpus)?;

        let temperature = get_temperature().ok();

        // 2026-09-02 representative-cpu-frequency
        // Reading every logical CPU's sysfs file dominated sensor-worker wakeups.
        // A first-available sample matches btop's low-cost `freq_mode=first` model.
        let frequency = first_available_cpu_frequency(logical_cpus, get_cpu_freq);

        Ok(Self {
            new_total_usage,
            new_thread_usages,
            temperature,
            frequency,
        })
    }
}

fn first_available_cpu_frequency(
    logical_cpus: usize,
    mut read_frequency: impl FnMut(usize) -> Result<u64>,
) -> Option<u64> {
    (0..logical_cpus).find_map(|core| read_frequency(core).ok())
}

#[derive(Debug, Clone, Default)]
pub struct CpuInfo {
    pub model_name: Option<String>,
    pub architecture: Option<String>,
    pub logical_cpus: Option<usize>,
    pub physical_cpus: Option<usize>,
    pub sockets: Option<usize>,
    pub virtualization: Option<String>,
    pub max_speed: Option<f64>,
}

fn trade_mark_symbols<S: AsRef<str>>(s: S) -> String {
    s.as_ref()
        .replace("(R)", "®")
        .replace("(tm)", "™")
        .replace("(TM)", "™")
}

/// Returns a `CPUInfo` struct populated with values gathered from `lscpu`.
///
/// # Errors
///
/// Will return `Err` if the are problems during reading or parsing
/// of the `lscpu` command
pub fn cpu_info() -> Result<CpuInfo> {
    let lscpu_output = String::from_utf8(
        std::process::Command::new("lscpu")
            .env("LC_ALL", "C")
            .output()
            .context("unable to run lscpu, is util-linux installed?")?
            .stdout,
    )
    .context("unable to parse lscpu output to UTF-8")?;

    let model_name = RE_LSCPU_MODEL_NAME
        .captures(&lscpu_output)
        .and_then(|captures| {
            captures
                .get(1)
                .map(|capture| trade_mark_symbols(capture.as_str()))
        });

    let architecture = RE_LSCPU_ARCHITECTURE
        .captures(&lscpu_output)
        .and_then(|captures| captures.get(1).map(|capture| capture.as_str().into()));

    let sockets = RE_LSCPU_SOCKETS
        .captures(&lscpu_output)
        .and_then(|captures| {
            captures
                .get(1)
                .and_then(|capture| capture.as_str().parse().ok())
        });

    let logical_cpus = RE_LSCPU_CPUS.captures(&lscpu_output).and_then(|captures| {
        captures
            .get(1)
            .and_then(|capture| capture.as_str().parse().ok())
    });

    let physical_cpus = RE_LSCPU_CORES.captures(&lscpu_output).and_then(|captures| {
        captures
            .get(1)
            .and_then(|capture| capture.as_str().parse::<usize>().ok())
            .map(|int| int * sockets.unwrap_or(1))
    });

    let virtualization = RE_LSCPU_VIRTUALIZATION
        .captures(&lscpu_output)
        .and_then(|captures| captures.get(1).map(|capture| capture.as_str().into()));

    let max_speed = RE_LSCPU_MAX_MHZ
        .captures(&lscpu_output)
        .and_then(|captures| {
            captures.get(1).and_then(|capture| {
                capture
                    .as_str()
                    .parse::<f64>()
                    .ok()
                    .map(|float| float * 1_000_000.0)
            })
        });

    Ok(CpuInfo {
        model_name,
        architecture,
        logical_cpus,
        physical_cpus,
        sockets,
        virtualization,
        max_speed,
    })
}

/// Returns the frequency of the given CPU `core`
///
/// # Errors
///
/// Will return `Err` if the are problems during reading or parsing
/// of the corresponding file in sysfs
pub fn get_cpu_freq(core: usize) -> Result<u64> {
    std::fs::read_to_string(format!(
        "/sys/devices/system/cpu/cpu{core}/cpufreq/scaling_cur_freq"
    ))
    .with_context(|| format!("unable to read scaling_cur_freq for core {core}"))?
    .trim()
    .parse::<u64>()
    .context("can't parse scaling_cur_freq to usize")
    .map(|x| x * 1000)
}

/// Parse one `/proc/stat` cpu line into `(idle_time, total_time)`.
/// Layout: cpuN user nice system idle iowait irq softirq steal guest guest_nice
fn parse_proc_stat_line(line: &str) -> Result<(u64, u64)> {
    let mut parts = line.split_whitespace();
    let _label = parts.next().context("empty /proc/stat line")?;

    let mut values = [0u64; 10];
    let mut count = 0usize;
    for (slot, token) in values.iter_mut().zip(parts) {
        *slot = token
            .parse::<u64>()
            .context("unable to parse CPU times from /proc/stat")?;
        count += 1;
    }
    if count < 4 {
        bail!("not enough fields in /proc/stat cpu line");
    }

    // idle + iowait (fields 4 and 5, 0-indexed 3 and 4)
    let idle_time = values[3].saturating_add(values[4]);
    let sum: u64 = values[..count].iter().sum();
    Ok((idle_time, sum))
}

/// Read `/proc/stat` once and return total + per-core usage.
fn read_all_cpu_usages(logical_cpus: usize) -> Result<((u64, u64), Vec<(u64, u64)>)> {
    let proc_stat = std::fs::read_to_string("/proc/stat").context("unable to read /proc/stat")?;

    let mut total = None;
    let mut threads = Vec::with_capacity(logical_cpus);

    for line in proc_stat.lines() {
        if !line.starts_with("cpu") {
            // All cpu* lines are at the top; stop once we leave them.
            if total.is_some() {
                break;
            }
            continue;
        }

        // "cpu " (aggregate) vs "cpu0", "cpu1", ...
        let after_cpu = &line[3..];
        if after_cpu.starts_with(' ') || after_cpu.starts_with('\t') {
            total = Some(parse_proc_stat_line(line)?);
        } else if after_cpu.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            if threads.len() < logical_cpus {
                threads.push(parse_proc_stat_line(line)?);
            }
        }
    }

    let total = total.context("missing aggregate cpu line in /proc/stat")?;
    if threads.len() < logical_cpus {
        bail!(
            "expected {logical_cpus} cpu cores in /proc/stat, got {}",
            threads.len()
        );
    }
    threads.truncate(logical_cpus);

    Ok((total, threads))
}

/// Returns the CPU usage of either all cores combined (if supplied argument is `None`),
/// or of a specific thread (taken from the supplied argument starting at 0)
/// Please keep in mind that this is the total CPU time since boot, you have to do delta
/// calculations yourself. The tuple's layout is: `(idle_time, total_time)`
///
/// # Errors
///
/// Will return `Err` if the are problems during reading or parsing
/// of /proc/stat
#[allow(dead_code)]
pub fn get_cpu_usage(core: Option<usize>) -> Result<(u64, u64)> {
    let n = core.map_or(0, |c| c + 1);
    let (total, threads) = read_all_cpu_usages(n.max(1))?;
    match core {
        None => Ok(total),
        Some(i) => threads
            .into_iter()
            .nth(i)
            .context("`core` argument greater than amount of cores"),
    }
}

/// Returns the CPU temperature.
///
/// # Errors
///
/// Will return `Err` if there was no way to read the CPU temperature.
pub fn get_temperature() -> Result<f32> {
    if let Some(path) = CPU_TEMPERATURE_PATH.as_ref() {
        read_sysfs_thermal(path)
    } else {
        bail!("no CPU temperature sensor found")
    }
}

fn read_sysfs_thermal<P: AsRef<Path>>(path: P) -> Result<f32> {
    let path = path.as_ref();
    let temp_string = std::fs::read_to_string(path)
        .with_context(|| format!("unable to read {}", path.display()))?;
    temp_string
        .trim()
        .parse::<f32>()
        .with_context(|| format!("unable to parse {}", path.display()))
        .map(|t| t / 1000f32)
}

#[cfg(test)]
mod tests {
    use anyhow::anyhow;

    use super::first_available_cpu_frequency;

    #[test]
    fn representative_frequency_stops_after_first_success() {
        let mut attempts = Vec::new();
        let frequency = first_available_cpu_frequency(4, |core| {
            attempts.push(core);
            Ok(2_400_000_000 + core as u64)
        });

        assert_eq!(frequency, Some(2_400_000_000));
        assert_eq!(attempts, vec![0]);
    }

    #[test]
    fn representative_frequency_skips_unavailable_cores() {
        let mut attempts = Vec::new();
        let frequency = first_available_cpu_frequency(4, |core| {
            attempts.push(core);
            if core < 2 {
                Err(anyhow!("offline"))
            } else {
                Ok(1_800_000_000)
            }
        });

        assert_eq!(frequency, Some(1_800_000_000));
        assert_eq!(attempts, vec![0, 1, 2]);
    }

    #[test]
    fn zero_logical_cpus_does_not_read_frequency() {
        let frequency = first_available_cpu_frequency(0, |_| panic!("reader must not run"));

        assert_eq!(frequency, None);
    }
}
