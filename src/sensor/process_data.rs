use anyhow::{Context, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    time::SystemTime,
};

use super::pci::PciSlot;

static USERS_CACHE: Lazy<HashMap<u32, String>> = Lazy::new(|| unsafe {
    uzers::all_users()
        .map(|user| (user.uid(), user.name().to_string_lossy().to_string()))
        .collect()
});

static PAGESIZE: Lazy<usize> = Lazy::new(sysconf::pagesize);
static RE_UID: Lazy<Regex> = Lazy::new(|| Regex::new(r"Uid:\s*(\d+)").unwrap());

/// The container runtime associated with a process.
#[derive(Debug, Clone, Copy, Default, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub enum Containerization {
    #[default]
    None,
    Flatpak,
    Snap,
}

/// Per-process GPU counters.
///
/// DRM counters are accumulated nanoseconds for AMD/Intel devices. NVIDIA
/// values are percentages from NVML and set `nvidia` to true. The process
/// collector currently leaves this map empty when the platform has no safe,
/// portable per-process GPU source.
#[derive(Debug, Clone, Copy, Default, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuUsageStats {
    pub gfx: u64,
    pub mem: u64,
    pub enc: u64,
    pub dec: u64,
    pub nvidia: bool,
}

/// Dynamic and cached metadata collected from one `/proc/<pid>` directory.
#[derive(Debug, Default, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessData {
    pub pid: i32,
    pub user: String,
    proc_path: PathBuf,
    pub comm: String,
    pub commandline: String,
    pub user_cpu_time: u64,
    pub system_cpu_time: u64,
    pub cpu_time_timestamp: u64,
    pub memory_usage: usize,
    /// Process start time in clock ticks, as documented by `proc(5)`.
    pub starttime: u64,
    pub cgroup: Option<String>,
    pub containerization: Containerization,
    pub read_bytes: Option<u64>,
    pub write_bytes: Option<u64>,
    pub timestamp: u64,
    /// GPU counters keyed by PCI slot.
    pub gpu_usage_stats: BTreeMap<PciSlot, GpuUsageStats>,
}

#[derive(Debug, Clone)]
struct CachedProcessStatic {
    user: String,
    comm: String,
    commandline: String,
    cgroup: Option<String>,
    containerization: Containerization,
}

/// Static procfs metadata keyed by the Linux process identity `(pid, starttime)`.
/// The start time prevents stale metadata from surviving PID recycling.
#[derive(Debug, Default)]
pub struct ProcessDataCache {
    entries: HashMap<(i32, u64), CachedProcessStatic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProcessStatSample {
    user_cpu_time: u64,
    system_cpu_time: u64,
    starttime: u64,
}

/// Stateful process collector owned by the sensor layer.
///
/// The reader and metadata cache intentionally live in one worker-owned value:
/// collecting processes from multiple threads would make both procfs races and
/// PID-reuse handling harder to reason about.
#[derive(Debug, Default)]
pub struct ProcessCollector {
    reader: ReuseReader,
    cache: ProcessDataCache,
}

impl ProcessCollector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn collect(&mut self) -> Result<Vec<ProcessData>> {
        ProcessData::all_process_data_cached(&mut self.reader, &mut self.cache)
    }

    #[cfg(test)]
    fn collect_from_root(&mut self, proc_root: &Path) -> Result<Vec<ProcessData>> {
        ProcessData::all_process_data_from_root(&mut self.reader, &mut self.cache, proc_root)
    }
}

fn parse_process_stat(stat: &str) -> Result<ProcessStatSample> {
    let (_, fields) = stat
        .rsplit_once(") ")
        .context("stat doesn't contain a closing command field")?;
    let mut fields = fields.split_whitespace();
    let user_cpu_time = fields
        .nth(11)
        .context("stat is missing user CPU time")?
        .parse()?;
    let system_cpu_time = fields
        .next()
        .context("stat is missing system CPU time")?
        .parse()?;
    let starttime = fields
        .nth(6)
        .context("stat is missing process start time")?
        .parse()?;

    Ok(ProcessStatSample {
        user_cpu_time,
        system_cpu_time,
        starttime,
    })
}

impl ProcessData {
    fn sanitize_cgroup<S: AsRef<str>>(cgroup: S) -> Option<String> {
        let cgroups_v2_line = cgroup.as_ref().split('\n').find(|s| s.starts_with("0::"))?;
        if cgroups_v2_line.ends_with(".scope") {
            let cgroups_segments: Vec<&str> = cgroups_v2_line.split('-').collect();
            if cgroups_segments.len() > 1 {
                cgroups_segments
                    .get(cgroups_segments.len() - 2)
                    .map(|s| unescape::unescape(s).unwrap_or_else(|| (*s).to_string()))
            } else {
                None
            }
        } else if cgroups_v2_line.ends_with(".service") {
            let cgroups_segments: Vec<&str> = cgroups_v2_line.split('/').collect();
            cgroups_segments.last().map(|last| {
                last[..last.len() - ".service".len()]
                    .split('@')
                    .next()
                    .map(|s| unescape::unescape(s).unwrap_or_else(|| s.to_string()))
                    .map(|s| {
                        if s.contains("dbus-:") {
                            s.split('-').last().unwrap_or(&s).to_string()
                        } else {
                            s
                        }
                    })
            })?
        } else {
            None
        }
    }

    fn get_uid(reader: &mut ReuseReader, proc_path: &Path) -> Result<u32> {
        reader.read(proc_path.join("status"), |s| {
            if let Some(captures) = RE_UID.captures(s) {
                captures
                    .get(1)
                    .context("no uid found")?
                    .as_str()
                    .parse::<u32>()
                    .context("couldn't parse uid in /status")
            } else {
                Ok(0)
            }
        })
    }

    pub fn all_process_data(reader: &mut ReuseReader) -> Result<Vec<Self>> {
        let mut cache = ProcessDataCache::default();
        Self::all_process_data_cached(reader, &mut cache)
    }

    pub fn all_process_data_cached(
        reader: &mut ReuseReader,
        cache: &mut ProcessDataCache,
    ) -> Result<Vec<Self>> {
        Self::all_process_data_from_root(reader, cache, Path::new("/proc"))
    }

    fn all_process_data_from_root(
        reader: &mut ReuseReader,
        cache: &mut ProcessDataCache,
        proc_root: &Path,
    ) -> Result<Vec<Self>> {
        let mut process_data = Vec::with_capacity(256);
        let mut seen = HashSet::with_capacity(cache.entries.len());

        for entry in fs::read_dir(proc_root)
            .context("unable to read procfs")?
            .flatten()
        {
            let Ok(pid) = entry.file_name().to_string_lossy().parse::<i32>() else {
                continue;
            };
            if let Ok(data) = Self::try_from_path_cached(reader, cache, entry.path(), pid) {
                seen.insert((pid, data.starttime));
                process_data.push(data);
            }
        }

        cache.entries.retain(|identity, _| seen.contains(identity));
        Ok(process_data)
    }

    pub fn try_from_path(reader: &mut ReuseReader, proc_path: PathBuf) -> Result<Self> {
        let pid = Self::pid_from_path(&proc_path)?;
        let mut cache = ProcessDataCache::default();
        Self::try_from_path_cached(reader, &mut cache, proc_path, pid)
    }

    fn pid_from_path(proc_path: &Path) -> Result<i32> {
        proc_path
            .file_name()
            .context("proc_path terminates in ..")?
            .to_str()
            .context("can't turn OsStr to str")?
            .parse()
            .context("proc path does not end in a PID")
    }

    fn read_static_data(reader: &mut ReuseReader, proc_path: &Path) -> Result<CachedProcessStatic> {
        let comm = reader.read(proc_path.join("comm"), |value| {
            Ok(value.trim_end_matches('\n').to_owned())
        })?;
        let commandline = reader.read_to_str(proc_path.join("cmdline"))?;
        let cgroup = reader.read(proc_path.join("cgroup"), |value| {
            Ok(Self::sanitize_cgroup(value))
        })?;
        let user = USERS_CACHE
            .get(&Self::get_uid(reader, proc_path)?)
            .cloned()
            .unwrap_or_else(|| String::from("root"));
        let containerization = if commandline.starts_with("/snap/") {
            Containerization::Snap
        } else if proc_path.join("root").join(".flatpak-info").exists() {
            Containerization::Flatpak
        } else {
            Containerization::None
        };

        Ok(CachedProcessStatic {
            user,
            comm,
            commandline,
            cgroup,
            containerization,
        })
    }

    fn try_from_path_cached(
        reader: &mut ReuseReader,
        cache: &mut ProcessDataCache,
        proc_path: PathBuf,
        pid: i32,
    ) -> Result<Self> {
        let stat = reader.read(proc_path.join("stat"), parse_process_stat)?;
        let identity = (pid, stat.starttime);
        if !cache.entries.contains_key(&identity) {
            let static_data = Self::read_static_data(reader, &proc_path)?;
            cache.entries.insert(identity, static_data);
        }
        let static_data = cache
            .entries
            .get(&identity)
            .context("static process cache insert failed")?
            .clone();

        let memory_usage = reader.read(proc_path.join("statm"), |statm| {
            let mut fields = statm.split_whitespace();
            fields.next();
            let resident = fields
                .next()
                .context("statm is missing resident pages")?
                .parse::<usize>()?;
            let shared = fields
                .next()
                .context("statm is missing shared pages")?
                .parse::<usize>()?;
            Ok(resident.saturating_sub(shared).saturating_mul(*PAGESIZE))
        })?;

        let io_read_write = reader.read_to_opt(proc_path.join("io"), parse_io_bytes);
        let timestamp = unix_as_millis();

        Ok(Self {
            pid,
            user: static_data.user,
            comm: static_data.comm,
            commandline: static_data.commandline,
            user_cpu_time: stat.user_cpu_time,
            system_cpu_time: stat.system_cpu_time,
            cpu_time_timestamp: timestamp,
            memory_usage,
            starttime: stat.starttime,
            cgroup: static_data.cgroup,
            proc_path,
            containerization: static_data.containerization,
            read_bytes: io_read_write.and_then(|values| values[0]),
            write_bytes: io_read_write.and_then(|values| values[1]),
            timestamp,
            gpu_usage_stats: BTreeMap::new(),
        })
    }
}

pub fn unix_as_millis() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Parse `read_bytes` and `write_bytes` from `/proc/*/io`.
fn parse_io_bytes(io: &str) -> Option<[Option<u64>; 2]> {
    let mut read_bytes = None;
    let mut write_bytes = None;
    for line in io.lines() {
        if read_bytes.is_none() {
            if let Some(value) = line
                .strip_prefix("read_bytes:")
                .and_then(|value| value.trim().parse().ok())
            {
                read_bytes = Some(value);
            }
        }
        if write_bytes.is_none() {
            if let Some(value) = line
                .strip_prefix("write_bytes:")
                .and_then(|value| value.trim().parse().ok())
            {
                write_bytes = Some(value);
            }
        }
        if read_bytes.is_some() && write_bytes.is_some() {
            break;
        }
    }
    Some([read_bytes, write_bytes])
}

/// Reuses one allocation while reading the many small procfs/sysfs files in a
/// sensor sample.
#[derive(Debug, Default)]
pub struct ReuseReader {
    buffer: String,
}

impl ReuseReader {
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn read<P: AsRef<Path>, T, F: FnOnce(&str) -> anyhow::Result<T>>(
        &mut self,
        filepath: P,
        mapper: F,
    ) -> anyhow::Result<T> {
        self.buffer.clear();
        let mut file = File::open(filepath.as_ref())?;
        file.read_to_string(&mut self.buffer)?;
        mapper(&self.buffer)
    }

    #[inline]
    pub fn read_to_opt<P: AsRef<Path>, T, F: FnOnce(&str) -> Option<T>>(
        &mut self,
        filepath: P,
        mapper: F,
    ) -> Option<T> {
        self.buffer.clear();
        let mut file = File::open(filepath.as_ref()).ok()?;
        file.read_to_string(&mut self.buffer).ok()?;
        mapper(&self.buffer)
    }

    #[inline]
    pub fn read_to_str<P: AsRef<Path>>(&mut self, filepath: P) -> anyhow::Result<String> {
        self.buffer.clear();
        let mut file = File::open(filepath.as_ref())?;
        file.read_to_string(&mut self.buffer)?;
        Ok(self.buffer.clone())
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf, time::SystemTime};

    use super::{parse_process_stat, ProcessCollector, ProcessData, ProcessDataCache, ReuseReader};

    struct TempProcRoot(PathBuf);

    impl TempProcRoot {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "restop-process-cache-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempProcRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn stat_line(pid: i32, comm: &str, starttime: u64, user: u64, system: u64) -> String {
        format!(
            "{pid} ({comm}) S 0 0 0 0 0 0 0 0 0 0 {user} {system} 0 0 20 0 1 0 {starttime} 4096 1\n"
        )
    }

    fn write_process(root: &std::path::Path, pid: i32, starttime: u64, comm: &str) {
        let process = root.join(pid.to_string());
        fs::create_dir_all(&process).unwrap();
        fs::write(process.join("stat"), stat_line(pid, comm, starttime, 11, 7)).unwrap();
        fs::write(process.join("statm"), "100 50 10 0 0 0 0\n").unwrap();
        fs::write(process.join("comm"), format!("{comm}\n")).unwrap();
        fs::write(process.join("cmdline"), format!("/usr/bin/{comm}\0")).unwrap();
        fs::write(process.join("cgroup"), "0::/system.slice/test.service\n").unwrap();
        fs::write(process.join("status"), "Name:\ttest\nUid:\t0\t0\t0\t0\n").unwrap();
        fs::write(process.join("io"), "read_bytes: 10\nwrite_bytes: 20\n").unwrap();
    }

    #[test]
    fn stat_parser_handles_closing_parenthesis_in_command() {
        let sample = parse_process_stat(&stat_line(42, "odd ) command", 98765, 11, 7)).unwrap();
        assert_eq!(sample.user_cpu_time, 11);
        assert_eq!(sample.system_cpu_time, 7);
        assert_eq!(sample.starttime, 98765);
    }

    #[test]
    fn static_metadata_is_cached_by_pid_and_starttime() {
        let root = TempProcRoot::new();
        let mut reader = ReuseReader::new();
        let mut cache = ProcessDataCache::default();
        write_process(&root.0, 42, 100, "first");

        let first =
            ProcessData::all_process_data_from_root(&mut reader, &mut cache, &root.0).unwrap();
        assert_eq!(first[0].comm, "first");
        assert_eq!(cache.entries.len(), 1);

        write_process(&root.0, 42, 100, "changed-on-disk");
        let cached =
            ProcessData::all_process_data_from_root(&mut reader, &mut cache, &root.0).unwrap();
        assert_eq!(cached[0].comm, "first");

        write_process(&root.0, 42, 101, "replacement");
        let replacement =
            ProcessData::all_process_data_from_root(&mut reader, &mut cache, &root.0).unwrap();
        assert_eq!(replacement[0].comm, "replacement");
        assert_eq!(replacement[0].starttime, 101);
        assert_eq!(cache.entries.len(), 1);

        fs::remove_dir_all(root.0.join("42")).unwrap();
        let empty =
            ProcessData::all_process_data_from_root(&mut reader, &mut cache, &root.0).unwrap();
        assert!(empty.is_empty());
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn collector_owns_sampling_state() {
        let root = TempProcRoot::new();
        write_process(&root.0, 42, 100, "first");

        let mut collector = ProcessCollector::new();
        let data = collector.collect_from_root(&root.0).unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].comm, "first");
    }

    #[test]
    fn io_parser_keeps_missing_counters_optional() {
        let mut reader = ReuseReader::new();
        let path = std::env::temp_dir().join(format!("restop-process-io-{}", std::process::id()));
        fs::write(&path, "read_bytes: 42\nmalformed\n").unwrap();
        let values = reader.read_to_opt(&path, super::parse_io_bytes).unwrap();
        assert_eq!(values, [Some(42), None]);
        let _ = fs::remove_file(path);
    }
}
