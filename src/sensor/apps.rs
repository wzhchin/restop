use hashbrown::{HashMap, HashSet};

use crate::tarits::NaNDefault;

use super::{
    pci::PciSlot,
    process::{Process, ProcessItem},
    process_data::{Containerization, ProcessData},
    TICK_RATE,
};

#[derive(Debug, Clone, Default)]
pub struct AppsContext {
    processes: HashMap<i32, Process>,
    processes_assigned_to_apps: HashSet<i32>,
    read_bytes_from_dead_system_processes: u64,
    write_bytes_from_dead_system_processes: u64,
}

/// Convenience struct for displaying running applications and
/// displaying a "System Processes" item.
#[derive(Debug, Clone)]
pub struct AppItem {
    pub id: Option<String>,
    pub display_name: String,
    pub description: Option<String>,
    pub memory_usage: usize,
    pub cpu_time_ratio: f32,
    pub processes_amount: usize,
    pub containerization: Containerization,
    pub running_since: String,
    pub read_speed: f64,
    pub read_total: u64,
    pub write_speed: f64,
    pub write_total: u64,
    pub gpu_usage: f32,
    pub enc_usage: f32,
    pub dec_usage: f32,
    pub gpu_mem_usage: u64,
}

impl AppsContext {
    pub fn new() -> AppsContext {
        AppsContext {
            processes: HashMap::new(),
            processes_assigned_to_apps: HashSet::new(),
            read_bytes_from_dead_system_processes: 0,
            write_bytes_from_dead_system_processes: 0,
        }
    }

    pub fn gpu_fraction(&self, pci_slot: PciSlot) -> f32 {
        self.all_processes()
            .map(|process| {
                (
                    &process.data.gpu_usage_stats,
                    &process.gpu_usage_stats_last,
                    process.data.timestamp,
                    process.timestamp_last,
                )
            })
            .map(|(new, old, timestamp, timestamp_last)| {
                (
                    new.get(&pci_slot),
                    old.get(&pci_slot),
                    timestamp,
                    timestamp_last,
                )
            })
            .filter_map(|(new, old, timestamp, timestamp_last)| match (new, old) {
                (Some(new), Some(old)) => Some((new, old, timestamp, timestamp_last)),
                _ => None,
            })
            .map(|(new, old, timestamp, timestamp_last)| {
                if new.nvidia {
                    new.gfx as f32 / 100.0
                } else if old.gfx == 0 {
                    0.0
                } else {
                    ((new.gfx.saturating_sub(old.gfx) as f32)
                        / (timestamp.saturating_sub(timestamp_last) as f32))
                        .nan_default(0.0)
                        / 1_000_000.0
                }
            })
            .sum()
    }

    pub fn encoder_fraction(&self, pci_slot: PciSlot) -> f32 {
        self.all_processes()
            .map(|process| {
                (
                    &process.data.gpu_usage_stats,
                    &process.gpu_usage_stats_last,
                    process.data.timestamp,
                    process.timestamp_last,
                )
            })
            .map(|(new, old, timestamp, timestamp_last)| {
                (
                    new.get(&pci_slot),
                    old.get(&pci_slot),
                    timestamp,
                    timestamp_last,
                )
            })
            .filter_map(|(new, old, timestamp, timestamp_last)| match (new, old) {
                (Some(new), Some(old)) => Some((new, old, timestamp, timestamp_last)),
                _ => None,
            })
            .map(|(new, old, timestamp, timestamp_last)| {
                if new.nvidia {
                    new.enc as f32 / 100.0
                } else if old.enc == 0 {
                    0.0
                } else {
                    ((new.enc.saturating_sub(old.enc) as f32)
                        / (timestamp.saturating_sub(timestamp_last) as f32))
                        .nan_default(0.0)
                        / 1_000_000.0
                }
            })
            .sum()
    }

    pub fn decoder_fraction(&self, pci_slot: PciSlot) -> f32 {
        self.all_processes()
            .map(|process| {
                (
                    &process.data.gpu_usage_stats,
                    &process.gpu_usage_stats_last,
                    process.data.timestamp,
                    process.timestamp_last,
                )
            })
            .map(|(new, old, timestamp, timestamp_last)| {
                (
                    new.get(&pci_slot),
                    old.get(&pci_slot),
                    timestamp,
                    timestamp_last,
                )
            })
            .filter_map(|(new, old, timestamp, timestamp_last)| match (new, old) {
                (Some(new), Some(old)) => Some((new, old, timestamp, timestamp_last)),
                _ => None,
            })
            .map(|(new, old, timestamp, timestamp_last)| {
                if new.nvidia {
                    new.dec as f32 / 100.0
                } else if old.dec == 0 {
                    0.0
                } else {
                    ((new.dec.saturating_sub(old.dec) as f32)
                        / (timestamp.saturating_sub(timestamp_last) as f32))
                        .nan_default(0.0)
                        / 1_000_000.0
                }
            })
            .sum()
    }

    pub fn get_process(&self, pid: i32) -> Option<&Process> {
        self.processes.get(&pid)
    }

    pub fn all_processes(&self) -> impl Iterator<Item = &Process> {
        self.processes.values()
    }

    pub fn all_processes_mut(&mut self) -> impl Iterator<Item = &mut Process> {
        self.processes.values_mut()
    }

    /// Returns running processes as display items.
    pub fn process_items(&self) -> HashMap<i32, ProcessItem> {
        self.process_items_vec()
            .into_iter()
            .map(|item| (item.pid, item))
            .collect()
    }

    /// Collect process items without the intermediate HashMap (hot path).
    pub fn process_items_vec(&self) -> Vec<ProcessItem> {
        let mut items = Vec::with_capacity(self.processes.len());
        for process in self.all_processes() {
            if let Some(item) = self.process_item(process.data.pid) {
                items.push(item);
            }
        }
        items
    }

    pub fn process_item(&self, pid: i32) -> Option<ProcessItem> {
        self.get_process(pid).map(|process| {
            let full_comm = if process.executable_name.starts_with(&process.data.comm) {
                process.executable_name.clone()
            } else {
                process.data.comm.clone()
            };
            ProcessItem {
                pid: process.data.pid,
                user: process.data.user.clone(),
                display_name: full_comm.clone(),
                memory_usage: process.data.memory_usage,
                cpu_time_ratio: process.cpu_time_ratio(),
                user_cpu_time: ((process.data.user_cpu_time) as f64 / (*TICK_RATE) as f64),
                system_cpu_time: ((process.data.system_cpu_time) as f64 / (*TICK_RATE) as f64),
                commandline: Process::sanitize_cmdline(process.data.commandline.clone())
                    .unwrap_or(full_comm),
                containerization: process.data.containerization,
                starttime: process.starttime(),
                starttime_ticks: process.data.starttime,
                cgroup: process.data.cgroup.clone(),
                read_speed: process.read_speed(),
                read_total: process.data.read_bytes,
                write_speed: process.write_speed(),
                write_total: process.data.write_bytes,
                gpu_usage: process.gpu_usage(),
                enc_usage: process.enc_usage(),
                dec_usage: process.dec_usage(),
                gpu_mem_usage: process.gpu_mem_usage(),
            }
        })
    }

    /// Refreshes the statistics about the running applications and processes.
    pub fn refresh(&mut self, new_process_data: Vec<ProcessData>) {
        let mut updated_processes = HashSet::new();

        for process_data in new_process_data {
            updated_processes.insert(process_data.pid);
            // refresh our old processes
            if let Some(old_process) = self.processes.get_mut(&process_data.pid) {
                old_process.update_from_process_data(process_data);
            } else {
                // this is a new process, see if it belongs to a graphical app

                let new_process = Process::from_process_data(process_data);

                self.processes.insert(new_process.data.pid, new_process);
            }
        }

        // same as above but for system processes
        let (read_dead, write_dead) = self
            .processes
            .iter()
            .filter(|(pid, _)| {
                !self.processes_assigned_to_apps.contains(*pid) && !updated_processes.contains(*pid)
            })
            .map(|(_, process)| (process.data.read_bytes, process.data.write_bytes))
            .filter_map(
                |(read_bytes, write_bytes)| match (read_bytes, write_bytes) {
                    (Some(read), Some(write)) => Some((read, write)),
                    _ => None,
                },
            )
            .reduce(|sum, current| (sum.0 + current.0, sum.1 + current.1))
            .unwrap_or((0, 0));
        self.read_bytes_from_dead_system_processes += read_dead;
        self.write_bytes_from_dead_system_processes += write_dead;

        // remove the dead process from our process map
        self.processes
            .retain(|pid, _| updated_processes.contains(pid));

        // remove the dead process from out list of app processes
        self.processes_assigned_to_apps
            .retain(|pid| updated_processes.contains(pid));
    }

    pub fn system_processes_iter(&self) -> impl Iterator<Item = &Process> {
        self.all_processes()
            .filter(|process| !self.processes_assigned_to_apps.contains(&process.data.pid))
    }
}
