use anyhow::{bail, Context, Result};
use chin_tools::AResult;
use nix::{
    sys::signal::{kill, Signal},
    unistd::Pid,
};
use once_cell::sync::Lazy;
use std::{
    collections::BTreeMap,
    fmt::Display,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
    thread,
};

use flume::{Receiver, Sender};

use log::debug;

use crate::{
    sensor::{apps::AppsContext, process_data::ProcessCollector},
    tarits::NaNDefault,
};

use super::{
    pci::PciSlot,
    process_data::{Containerization, GpuUsageStats, ProcessData},
    NUM_CPUS, TICK_RATE,
};

/// Consecutive-sample process CPU as a 0–1 fraction of all CPUs.
/// Missing previous (`cpu_time_last == 0`) or a zero time/counter delta is
/// finite `0.0`, never a non-finite value.
pub(crate) fn cpu_time_ratio_from_samples(
    cpu_time_last: u64,
    timestamp_last: u64,
    user_cpu_time: u64,
    system_cpu_time: u64,
    timestamp: u64,
    tick_rate: u64,
    num_cpus: u64,
) -> f32 {
    if cpu_time_last == 0 {
        return 0.0;
    }
    let delta_cpu_time = (user_cpu_time.saturating_add(system_cpu_time))
        .saturating_sub(cpu_time_last) as f32
        * 1000.0;
    let delta_time = timestamp.saturating_sub(timestamp_last);
    let denom = (delta_time
        .saturating_mul(tick_rate)
        .saturating_mul(num_cpus)) as f32;
    if denom == 0.0 {
        return 0.0;
    }
    let ratio = delta_cpu_time / denom;
    if ratio.is_finite() {
        ratio
    } else {
        0.0
    }
}

/// Represents a process that can be found within procfs.
#[derive(Debug, Clone, PartialEq)]
pub struct Process {
    pub data: ProcessData,
    pub executable_path: String,
    pub executable_name: String,
    pub cpu_time_last: u64,
    pub timestamp_last: u64,
    pub read_bytes_last: Option<u64>,
    pub write_bytes_last: Option<u64>,
    pub gpu_usage_stats_last: BTreeMap<PciSlot, GpuUsageStats>,
}

// TODO: Better name?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessAction {
    TERM,
    INT,
    HUP,
    STOP,
    KILL,
    CONT,
}

impl ProcessAction {
    pub fn signal(self) -> Signal {
        match self {
            ProcessAction::TERM => Signal::SIGTERM,
            ProcessAction::INT => Signal::SIGINT,
            ProcessAction::HUP => Signal::SIGHUP,
            ProcessAction::STOP => Signal::SIGSTOP,
            ProcessAction::KILL => Signal::SIGKILL,
            ProcessAction::CONT => Signal::SIGCONT,
        }
    }
}

pub fn send_process_action(pid: i32, action: ProcessAction) -> Result<()> {
    kill(Pid::from_raw(pid), action.signal())
        .with_context(|| format!("unable to send {:?} to process {pid}", action.signal()))
}
/// Convenience struct for displaying running processes
#[derive(Debug, Clone)]
pub struct ProcessItem {
    pub pid: i32,
    pub user: String,
    pub display_name: String,
    pub memory_usage: usize,
    pub cpu_time_ratio: f32,
    pub user_cpu_time: f64,
    pub system_cpu_time: f64,
    pub commandline: String,
    pub containerization: Containerization,
    pub starttime: f64,
    pub starttime_ticks: u64,
    pub cgroup: Option<String>,
    pub read_speed: Option<f64>,
    pub read_total: Option<u64>,
    pub write_speed: Option<f64>,
    pub write_total: Option<u64>,
    pub gpu_usage: f32,
    pub enc_usage: f32,
    pub dec_usage: f32,
    pub gpu_mem_usage: u64,
}

/// One internally consistent process sample for all process views.
#[derive(Debug, Clone)]
pub struct ProcessSnapshot {
    pub processes: Arc<Vec<ProcessItem>>,
    pub loadavg: Option<LoadAvg>,
    pub uptime: Option<u64>,
}

/// Process sensor state that must survive between samples so CPU and I/O
/// rates remain consecutive-sample values.
#[derive(Debug, Default)]
pub struct ProcessSensor {
    collector: ProcessCollector,
    app_context: AppsContext,
    loadavg: Option<LoadAvg>,
    uptime: Option<u64>,
}

impl ProcessSensor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Collect one process snapshot while retaining the previous valid
    /// uptime/load average and process state when an individual read fails.
    pub fn sample(&mut self) -> ProcessSnapshot {
        if let Ok(uptime) = read_proc_uptime() {
            self.uptime = Some(uptime);
        }
        if let Ok(loadavg) = read_proc_loadavg() {
            self.loadavg = Some(loadavg);
        }
        match self.collector.collect() {
            Ok(data) => self.app_context.refresh(data),
            Err(err) => log::error!("unable to update process data: {}", err),
        }

        ProcessSnapshot {
            processes: Arc::new(self.app_context.process_items_vec()),
            loadavg: self.loadavg.clone(),
            uptime: self.uptime,
        }
    }

    /// Start the sensor-owned process sampler. The callback is the
    /// application-layer adapter; this module does not depend on rendering or
    /// resource event types.
    pub fn spawn<F>(publish: F) -> AResult<()>
    where
        F: Fn(ProcessSnapshot) + Send + 'static,
    {
        thread::Builder::new()
            .name("processsensor".to_owned())
            .spawn(move || {
                let mut sensor = Self::new();
                while PROCESS_REQUESTS.receive().is_ok() {
                    publish(sensor.sample());
                }
            })?;

        Ok(())
    }
}

#[derive(Debug, Default)]
struct PendingProcessRequest {
    sample: bool,
}

/// Coalesces repeated process ticks into one pending sample without allowing
/// refresh storms to grow a queue.
#[derive(Debug)]
struct ProcessRequestQueue {
    wake_tx: Sender<()>,
    wake_rx: Receiver<()>,
    pending: Mutex<PendingProcessRequest>,
}

impl ProcessRequestQueue {
    fn new() -> Self {
        let (wake_tx, wake_rx) = flume::bounded(1);
        Self {
            wake_tx,
            wake_rx,
            pending: Mutex::new(PendingProcessRequest::default()),
        }
    }

    fn request(&self) {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        pending.sample = true;
        drop(pending);
        let _ = self.wake_tx.try_send(());
    }

    fn receive(&self) -> Result<(), flume::RecvError> {
        self.wake_rx.recv()?;
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _ = std::mem::take(&mut *pending);
        Ok(())
    }
}

static PROCESS_REQUESTS: Lazy<ProcessRequestQueue> = Lazy::new(ProcessRequestQueue::new);

/// Request the sensor-owned process sampler to produce its next snapshot.
pub fn request_sample() {
    PROCESS_REQUESTS.request();
}

const MAX_ENVIRONMENT_BYTES: usize = 64 * 1024;
const MAX_ENVIRONMENT_ENTRIES: usize = 128;

/// On-demand process metadata needed by the detail view.
#[derive(Debug, Clone)]
pub struct ProcessDetails {
    pub executable: String,
    pub workdir: String,
    pub environment: Vec<String>,
    pub environment_truncated: bool,
}

pub fn load_process_details(pid: i32) -> ProcessDetails {
    let proc_path = PathBuf::from("/proc").join(pid.to_string());
    let executable = read_link_or_unavailable(proc_path.join("exe"));
    let workdir = read_link_or_unavailable(proc_path.join("cwd"));
    let (environment, environment_truncated) = read_process_environment(&proc_path);

    ProcessDetails {
        executable,
        workdir,
        environment,
        environment_truncated,
    }
}

fn read_link_or_unavailable(path: PathBuf) -> String {
    std::fs::read_link(path)
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|error| format!("<unavailable: {error}>"))
}

pub(crate) fn parse_environment(bytes: &[u8], max_entries: usize) -> (Vec<String>, bool) {
    let mut entries: Vec<String> = bytes
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .take(max_entries.saturating_add(1))
        .map(|entry| String::from_utf8_lossy(entry).into_owned())
        .collect();
    let truncated = entries.len() > max_entries;
    entries.truncate(max_entries);
    entries.sort_unstable();
    (entries, truncated)
}

fn read_process_environment(proc_path: &Path) -> (Vec<String>, bool) {
    let result = (|| -> std::io::Result<(Vec<String>, bool)> {
        let file = File::open(proc_path.join("environ"))?;
        let mut bytes = Vec::with_capacity(MAX_ENVIRONMENT_BYTES.min(4096));
        file.take(MAX_ENVIRONMENT_BYTES.saturating_add(1) as u64)
            .read_to_end(&mut bytes)?;
        let bytes_truncated = bytes.len() > MAX_ENVIRONMENT_BYTES;
        bytes.truncate(MAX_ENVIRONMENT_BYTES);
        let (entries, entries_truncated) = parse_environment(&bytes, MAX_ENVIRONMENT_ENTRIES);
        Ok((entries, bytes_truncated || entries_truncated))
    })();

    result.unwrap_or_else(|error| (vec![format!("<unavailable: {error}>")], false))
}

fn parse_proc_starttime(stat: &str) -> Option<u64> {
    let (_, fields) = stat.rsplit_once(") ")?;
    fields.split_whitespace().nth(19)?.parse().ok()
}

pub fn read_process_starttime_ticks(pid: i32) -> std::io::Result<Option<u64>> {
    std::fs::read_to_string(PathBuf::from("/proc").join(pid.to_string()).join("stat"))
        .map(|stat| parse_proc_starttime(&stat))
}

impl Process {
    pub fn from_process_data(process_data: ProcessData) -> Self {
        let executable_path = process_data
            .commandline
            .split('\0')
            .nth(0)
            .and_then(|nul_split| nul_split.split(" --").nth(0)) // chromium (and thus everything based on it) doesn't use \0 as delimiter
            .unwrap_or(&process_data.commandline)
            .to_string();

        let executable_name = executable_path
            .split('/')
            .nth_back(0)
            .unwrap_or(&process_data.commandline)
            .to_string();

        let read_bytes_last = if process_data.read_bytes.is_some() {
            Some(0)
        } else {
            None
        };

        let write_bytes_last = if process_data.write_bytes.is_some() {
            Some(0)
        } else {
            None
        };

        Self {
            executable_path,
            executable_name,
            data: process_data,
            cpu_time_last: 0,
            timestamp_last: 0,
            read_bytes_last,
            write_bytes_last,
            gpu_usage_stats_last: Default::default(),
        }
    }

    pub fn update_from_process_data(&mut self, process_data: ProcessData) {
        if self.data.pid != process_data.pid || self.data.starttime != process_data.starttime {
            *self = Self::from_process_data(process_data);
            return;
        }

        self.cpu_time_last = self
            .data
            .user_cpu_time
            .saturating_add(self.data.system_cpu_time);
        self.timestamp_last = self.data.timestamp;
        self.read_bytes_last = self.data.read_bytes;
        self.write_bytes_last = self.data.write_bytes;
        self.gpu_usage_stats_last = std::mem::take(&mut self.data.gpu_usage_stats);
        self.data = process_data;
    }

    pub fn execute_process_action(&self, action: ProcessAction) -> Result<()> {
        send_process_action(self.data.pid, action)
    }

    #[allow(dead_code)]
    fn pkexec_execute_process_action(&self, action: &str, kill_path: &str) -> Result<()> {
        let status_code = Command::new("pkexec")
            .args([
                "--disable-internal-agent",
                kill_path,
                action,
                self.data.pid.to_string().as_str(),
            ])
            .output()?
            .status
            .code()
            .context("no status code?")?;

        if status_code == 0 || status_code == 3 {
            // 0 := successful; 3 := process not found which we don't care
            // about because that might happen because we killed the
            // process' parent first, killing the child before we explicitly do
            debug!(
                "Successfully {action}ed {} with elevated privileges",
                self.data.pid
            );
            Ok(())
        } else {
            bail!(
                "couldn't kill {} with elevated privileges due to unknown reasons, status code: {}",
                self.data.pid,
                status_code
            )
        }
    }

    #[must_use]
    pub fn cpu_time_ratio(&self) -> f32 {
        cpu_time_ratio_from_samples(
            self.cpu_time_last,
            self.timestamp_last,
            self.data.user_cpu_time,
            self.data.system_cpu_time,
            self.data.timestamp,
            *TICK_RATE as u64,
            *NUM_CPUS as u64,
        )
    }

    #[must_use]
    pub fn read_speed(&self) -> Option<f64> {
        if let (Some(read_bytes), Some(read_bytes_last)) =
            (self.data.read_bytes, self.read_bytes_last)
        {
            if self.timestamp_last == 0 {
                Some(0.0)
            } else {
                let bytes_delta = read_bytes.saturating_sub(read_bytes_last) as f64;
                let time_delta = self.data.timestamp.saturating_sub(self.timestamp_last) as f64;
                Some((bytes_delta / time_delta) * 1000.0)
            }
        } else {
            None
        }
    }

    #[must_use]
    pub fn write_speed(&self) -> Option<f64> {
        if let (Some(write_bytes), Some(write_bytes_last)) =
            (self.data.write_bytes, self.write_bytes_last)
        {
            if self.timestamp_last == 0 {
                Some(0.0)
            } else {
                let bytes_delta = write_bytes.saturating_sub(write_bytes_last) as f64;
                let time_delta = self.data.timestamp.saturating_sub(self.timestamp_last) as f64;
                Some((bytes_delta / time_delta) * 1000.0)
            }
        } else {
            None
        }
    }

    #[must_use]
    pub fn gpu_usage(&self) -> f32 {
        let mut returned_gpu_usage = 0.0;
        for (gpu, usage) in &self.data.gpu_usage_stats {
            if let Some(old_usage) = self.gpu_usage_stats_last.get(gpu) {
                let this_gpu_usage = if usage.nvidia {
                    usage.gfx as f32 / 100.0
                } else if old_usage.gfx == 0 {
                    0.0
                } else {
                    ((usage.gfx.saturating_sub(old_usage.gfx) as f32)
                        / (self.data.timestamp.saturating_sub(self.timestamp_last) as f32)
                            .nan_default(0.0))
                        / 1_000_000.0
                };

                if this_gpu_usage > returned_gpu_usage {
                    returned_gpu_usage = this_gpu_usage;
                }
            }
        }

        returned_gpu_usage
    }

    #[must_use]
    pub fn enc_usage(&self) -> f32 {
        let mut returned_gpu_usage = 0.0;
        for (gpu, usage) in &self.data.gpu_usage_stats {
            if let Some(old_usage) = self.gpu_usage_stats_last.get(gpu) {
                let this_gpu_usage = if usage.nvidia {
                    usage.enc as f32 / 100.0
                } else if old_usage.enc == 0 {
                    0.0
                } else {
                    ((usage.enc.saturating_sub(old_usage.enc) as f32)
                        / (self.data.timestamp.saturating_sub(self.timestamp_last) as f32)
                            .nan_default(0.0))
                        / 1_000_000.0
                };

                if this_gpu_usage > returned_gpu_usage {
                    returned_gpu_usage = this_gpu_usage;
                }
            }
        }

        returned_gpu_usage
    }

    #[must_use]
    pub fn dec_usage(&self) -> f32 {
        let mut returned_gpu_usage = 0.0;
        for (gpu, usage) in &self.data.gpu_usage_stats {
            if let Some(old_usage) = self.gpu_usage_stats_last.get(gpu) {
                let this_gpu_usage = if usage.nvidia {
                    usage.dec as f32 / 100.0
                } else if old_usage.dec == 0 {
                    0.0
                } else {
                    ((usage.dec.saturating_sub(old_usage.dec) as f32)
                        / (self.data.timestamp.saturating_sub(self.timestamp_last) as f32)
                            .nan_default(0.0))
                        / 1_000_000.0
                };

                if this_gpu_usage > returned_gpu_usage {
                    returned_gpu_usage = this_gpu_usage;
                }
            }
        }

        returned_gpu_usage
    }

    #[must_use]
    pub fn gpu_mem_usage(&self) -> u64 {
        self.data
            .gpu_usage_stats
            .values()
            .map(|stats| stats.mem)
            .sum()
    }

    #[must_use]
    pub fn starttime(&self) -> f64 {
        self.data.starttime as f64 / *TICK_RATE as f64
    }

    pub fn sanitize_cmdline<S: AsRef<str>>(cmdline: S) -> Option<String> {
        let cmdline = cmdline.as_ref();
        if cmdline.is_empty() {
            None
        } else {
            Some(cmdline.replace('\0', " "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        cpu_time_ratio_from_samples, parse_environment, parse_proc_starttime, Process,
        ProcessAction, ProcessRequestQueue,
    };
    use crate::sensor::process_data::ProcessData;
    use nix::sys::signal::Signal;

    fn process_data(pid: i32, starttime: u64, user_cpu_time: u64, timestamp: u64) -> ProcessData {
        let mut data = ProcessData::default();
        data.pid = pid;
        data.starttime = starttime;
        data.user_cpu_time = user_cpu_time;
        data.timestamp = timestamp;
        data
    }

    #[test]
    fn process_actions_map_to_exact_signals() {
        assert_eq!(ProcessAction::TERM.signal(), Signal::SIGTERM);
        assert_eq!(ProcessAction::INT.signal(), Signal::SIGINT);
        assert_eq!(ProcessAction::HUP.signal(), Signal::SIGHUP);
        assert_eq!(ProcessAction::STOP.signal(), Signal::SIGSTOP);
        assert_eq!(ProcessAction::KILL.signal(), Signal::SIGKILL);
        assert_eq!(ProcessAction::CONT.signal(), Signal::SIGCONT);
    }

    #[test]
    fn process_cpu_zero_denominator_is_finite_zero() {
        let same_ts = cpu_time_ratio_from_samples(10, 1000, 20, 5, 1000, 100, 1);
        assert_eq!(same_ts, 0.0);
        assert!(same_ts.is_finite());

        let missing_prev = cpu_time_ratio_from_samples(0, 0, 20, 5, 2000, 100, 1);
        assert_eq!(missing_prev, 0.0);
        assert!(missing_prev.is_finite());

        let zero_tick = cpu_time_ratio_from_samples(10, 1000, 20, 5, 2000, 0, 1);
        assert_eq!(zero_tick, 0.0);
        assert!(zero_tick.is_finite());
    }

    #[test]
    fn process_cpu_consecutive_delta_is_finite() {
        let ratio = cpu_time_ratio_from_samples(100, 1000, 200, 0, 2000, 100, 1);
        assert!(ratio.is_finite());
        assert!(ratio > 0.0);
    }

    #[test]
    fn pid_reuse_resets_previous_samples() {
        let mut process = Process::from_process_data(process_data(42, 100, 10, 1_000));
        process.update_from_process_data(process_data(42, 100, 20, 2_000));
        assert_eq!(process.cpu_time_last, 10);
        assert_eq!(process.timestamp_last, 1_000);

        process.update_from_process_data(process_data(42, 101, 5, 3_000));
        assert_eq!(process.data.starttime, 101);
        assert_eq!(process.cpu_time_last, 0);
        assert_eq!(process.timestamp_last, 0);
    }

    #[test]
    fn duplicate_process_samples_share_one_wake() {
        let queue = ProcessRequestQueue::new();
        queue.request();
        queue.request();
        queue.request();

        assert_eq!(queue.wake_rx.len(), 1);
        queue.receive().unwrap();
        assert!(queue.wake_rx.is_empty());
    }

    #[test]
    fn parses_nul_separated_environment() {
        let (entries, truncated) = parse_environment(b"B=two\0A=one\0", 8);
        assert_eq!(entries, vec!["A=one", "B=two"]);
        assert!(!truncated);
    }

    #[test]
    fn bounds_environment_entries() {
        let (entries, truncated) = parse_environment(b"A=1\0B=2\0C=3\0", 2);
        assert_eq!(entries, vec!["A=1", "B=2"]);
        assert!(truncated);
    }

    #[test]
    fn converts_non_utf8_environment_lossily() {
        let (entries, truncated) = parse_environment(b"A=\xff\0", 8);
        assert_eq!(entries, vec!["A=�"]);
        assert!(!truncated);
    }

    #[test]
    fn parses_environment_without_final_nul() {
        let (entries, truncated) = parse_environment(b"A=one\0B=two", 8);
        assert_eq!(entries, vec!["A=one", "B=two"]);
        assert!(!truncated);
    }

    #[test]
    fn parses_proc_starttime_after_parenthesized_command() {
        let stat =
            "42 (command with ) paren) S 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 98765 20";
        assert_eq!(parse_proc_starttime(stat), Some(98765));
    }
}

#[derive(Debug, Clone, Default)]
pub struct LoadAvg {
    pub last1: f32,
    pub last5: f32,
    pub last15: f32,
    pub processes: String,
}

impl Display for LoadAvg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:.1} {:.1} {:.1}",
            &self.last1, &self.last5, &self.last15
        )
    }
}

/// `proc/loadavg`  
/// The first three fields in this file are load average figures giving the number of jobs in the run queue (state R) or waiting for disk I/O (state D) averaged over 1, 5, and 15 minutes. They are the same as the load average numbers given by uptime(1) and other programs. The fourth field consists of two numbers separated by a slash (/). The first of these is the number of currently runnable kernel scheduling entities (processes, threads). The value after the slash is the number of kernel scheduling entities that currently exist on the system. The fifth field is the PID of the process that was most recently created on the system.
pub fn read_proc_loadavg() -> AResult<LoadAvg> {
    let s = std::fs::read_to_string("/proc/loadavg")?;
    let mut iter = s.split(" ");
    Ok(LoadAvg {
        last1: iter.next().context("first")?.parse::<f32>()?,
        last5: iter.next().context("second")?.parse::<f32>()?,
        last15: iter.next().context("thirs")?.parse::<f32>()?,
        processes: iter.next().context("Unable to read threads")?.to_string(),
    })
}

pub fn read_proc_uptime() -> AResult<u64> {
    std::fs::read_to_string("/proc/uptime")
        .context("unable to read /proc/uptime")
        .and_then(|procfs| {
            procfs
                .split(' ')
                .next()
                .map(str::to_string)
                .context("unable to split /proc/uptime")
        })
        .and_then(|uptime_str| {
            uptime_str
                .parse::<f64>()
                .context("unable to parse /proc/uptime")
        })
        .map(|uptime_secs: f64| uptime_secs as u64)
}
