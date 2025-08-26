use anyhow::{bail, Context, Result};
use chin_tools::AResult;
use process_data::{pci_slot::PciSlot, Containerization, GpuUsageStats, ProcessData};
use std::{collections::BTreeMap, fmt::Display, process::Command};

use log::debug;

use crate::tarits::NaNDefault;

use super::{NUM_CPUS, TICK_RATE};

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
    STOP,
    KILL,
    CONT,
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

    pub fn execute_process_action(&self, _action: ProcessAction) -> Result<()> {
        Ok(())
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
        if self.cpu_time_last == 0 {
            0.0
        } else {
            let delta_cpu_time = (self
                .data
                .user_cpu_time
                .saturating_add(self.data.system_cpu_time))
            .saturating_sub(self.cpu_time_last) as f32
                * 1000.0;
            let delta_time = self.data.timestamp.saturating_sub(self.timestamp_last);

            delta_cpu_time / (delta_time * *TICK_RATE as u64 * *NUM_CPUS as u64) as f32
        }
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
