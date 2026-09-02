use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime},
};

use chin_tools::AResult;
use once_cell::sync::Lazy;
use ratatui::text::{Line, Span};

use crate::{
    component::{
        grouped_lines::GroupedLines,
        ls_history_graph, s_percent_graph,
        stateful_lines::{StatefulGroupedLines, StatefulLinesType},
    },
    ring::{Ring, DEFAULT_HISTORY_LEN, SHORT_HISTORY_LEN},
    sensor::{
        drive::{DiskStats, Drive, DriveData, Partition},
        units::{convert_speed, convert_storage},
        Sensor,
    },
    tarits::{format_fraction_as_percent, None2NaN, None2NaNDef, None2NanString},
    view::theme::SharedTheme,
    view::{BlockArg, DetailArg},
};

use super::{Resource, SensorResultType, SensorRsp};

/// Drive activity history is stored as 0–1 fractions; graph max matches that unit.
pub(crate) const DRIVE_ACTIVITY_GRAPH_MAX: f64 = 1.0;

#[derive(Debug, Clone, Copy)]
pub(crate) enum DriveSpeedKind {
    Read,
    Write,
}

/// Pair a drive speed graph with the matching history and max.
pub(crate) fn drive_speed_graph_inputs<'a>(
    kind: DriveSpeedKind,
    read_history: &'a Ring<f64>,
    read_highest: f64,
    write_history: &'a Ring<f64>,
    write_highest: f64,
) -> (&'a Ring<f64>, f64) {
    match kind {
        DriveSpeedKind::Read => (read_history, read_highest),
        DriveSpeedKind::Write => (write_history, write_highest),
    }
}

/// Mount table is shared across drives; refresh at most every 5s.
static PARTITIONS_CACHE: Lazy<Mutex<(Instant, Vec<Partition>)>> =
    Lazy::new(|| Mutex::new((Instant::now() - Duration::from_secs(60), Vec::new())));

fn partitions_snapshot() -> Option<Vec<Partition>> {
    let mut guard = PARTITIONS_CACHE.lock().ok()?;
    let (last, cache) = &mut *guard;
    if last.elapsed() >= Duration::from_secs(5) || cache.is_empty() {
        if let Ok(parts) = Partition::fetch() {
            *cache = parts;
            *last = Instant::now();
        }
    }
    // Cheap clone of cached rows; expensive statvfs only every 5s.
    if cache.is_empty() {
        None
    } else {
        Some(cache.clone())
    }
}

#[derive(Debug)]
pub struct ResDrive {
    supply_name: String,
    id: String,
    info: Drive,
    /// Cached for sensor requests (avoids Arc+PathBuf alloc every tick).
    sysfs_path: Arc<PathBuf>,
    capacity: Option<u64>,

    // Show
    theme: SharedTheme,
    activity_history: Ring<f64>,

    read_speed_history: Ring<f64>,
    read_highest: Cell<f64>,
    read_total: Cell<f64>,

    write_speed_history: Ring<f64>,
    write_highest: Cell<f64>,
    write_total: Cell<f64>,

    is_virtual: Option<bool>,
    writiable: Option<bool>,
    removeable: Option<bool>,
    old_stats: RefCell<Option<DiskStats>>,

    last_timestamp: Cell<SystemTime>,

    partitions: Vec<Partition>,

    viewer_state: StatefulGroupedLines<'static>,
}

impl ResDrive {
    const SECTOR_SIZE: usize = 512;

    pub fn new(theme: SharedTheme) -> AResult<Vec<Self>> {
        let drive_paths = Drive::get_sysfs_paths().unwrap_or_default();
        Ok(drive_paths
            .iter()
            .filter_map(|dp| {
                let d = DriveData::new(dp);
                if d.is_virtual {
                    None
                } else {
                    let capacity = d.inner.capacity().ok();
                    Some(Self {
                        supply_name: d.inner.block_device.clone(),
                        id: d.inner.sysfs_path.as_path().to_string_lossy().to_string(),
                        sysfs_path: Arc::new(d.inner.sysfs_path.clone()),
                        theme: theme.clone(),
                        activity_history: Ring::new(DEFAULT_HISTORY_LEN),
                        info: d.inner,
                        is_virtual: None,
                        writiable: None,
                        removeable: None,
                        last_timestamp: Cell::new(SystemTime::now()),
                        old_stats: RefCell::new(None),
                        read_speed_history: Ring::new(SHORT_HISTORY_LEN),
                        read_highest: Default::default(),
                        read_total: Default::default(),
                        write_speed_history: Ring::new(SHORT_HISTORY_LEN),
                        write_highest: Default::default(),
                        write_total: Default::default(),
                        capacity,
                        partitions: vec![],
                        viewer_state: Default::default(),
                    })
                }
            })
            .collect())
    }

    fn activity_graph(&self, width: u16) -> Vec<Line<'static>> {
        ls_history_graph(
            width,
            &self.activity_history,
            DRIVE_ACTIVITY_GRAPH_MAX,
            0.,
            3,
            ratatui::style::Color::Black,
        )
    }

    fn update_drive_data(&mut self, data: &DriveData) {
        let DriveData {
            inner: _,
            is_virtual,
            writable,
            removable,
            disk_stats,
            capacity: _,
        } = data;

        self.is_virtual.replace(*is_virtual);
        self.writiable = writable.as_ref().ok().copied();
        self.removeable = removable.as_ref().ok().copied();

        let time_passed = SystemTime::now()
            .duration_since(self.last_timestamp.get())
            .map_or(1.0f64, |timestamp| timestamp.as_secs_f64())
            .max(0.001);
        self.last_timestamp.set(SystemTime::now());

        if let Some(old) = *self.old_stats.borrow() {
            let delta_read_ticks = disk_stats.read_ticks.saturating_sub(old.read_ticks);
            let delta_write_ticks = disk_stats.write_ticks.saturating_sub(old.write_ticks);
            let read_ratio = delta_read_ticks as f64 / (time_passed * 1000.0);
            let write_ratio = delta_write_ticks as f64 / (time_passed * 1000.0);
            let total_usage = f64::max(read_ratio, write_ratio).clamp(0.0, 1.0);
            self.activity_history.insert_at_first(total_usage);

            let delta_read_sectors = disk_stats.read_sectors.saturating_sub(old.read_sectors);
            let read_speed = (delta_read_sectors * Self::SECTOR_SIZE) as f64 / time_passed;
            self.read_total
                .set((disk_stats.read_sectors * Self::SECTOR_SIZE) as f64);
            self.read_speed_history.insert_at_first(read_speed);
            if read_speed > self.read_highest.get() {
                self.read_highest.set(read_speed);
            }

            let delta_write_sectors = disk_stats
                .write_sectors
                .saturating_sub(old.write_sectors);
            let write_speed = (delta_write_sectors * Self::SECTOR_SIZE) as f64 / time_passed;
            self.write_total
                .set((disk_stats.write_sectors * Self::SECTOR_SIZE) as f64);
            self.write_speed_history.insert_at_first(write_speed);
            if write_speed > self.write_highest.get() {
                self.write_highest.set(write_speed);
            }
        }

        self.old_stats.replace(Some(*disk_stats));
    }

    pub fn update_partition(&mut self, partitions: &[Partition]) {
        self.partitions = partitions
            .iter()
            .filter(|e| e.contains(self.info.block_device.as_str()))
            .cloned()
            .collect();
    }
}

#[derive(Debug)]
pub struct ResDriveRsp {
    pub data: DriveData,
    partitions: Option<Vec<Partition>>,
}

impl Resource for ResDrive {
    type Req = Arc<PathBuf>;

    type Rsp = ResDriveRsp;

    fn get_id(&self) -> &str {
        &self.id
    }

    fn get_req(&self) -> Self::Req {
        Arc::clone(&self.sysfs_path)
    }

    fn do_sensor(req: Self::Req) -> AResult<SensorResultType> {
        // Poll path only re-reads hot stats; model/type stay from construction.
        // Partitions are cached globally (5s) so N drives don't re-statvfs every tick.
        let data = DriveData::poll_stats(&req);
        Ok(SensorResultType::SyncResult(
            SensorRsp::Drive(ResDriveRsp {
                data,
                partitions: partitions_snapshot(),
            })
            .into(),
        ))
    }

    fn update_data(&mut self, data: &Self::Rsp) {
        self.update_drive_data(&data.data);
        if let Some(partitions) = data.partitions.as_ref() {
            self.update_partition(partitions);
        }
    }

    fn block(&self, args: &mut BlockArg) -> AResult<GroupedLines<'static>> {
        let width = args.width;
        let block = GroupedLines::builder(width, &self.theme)
            .kv("Size", self.info.display_name())
            .kv(
                "UR",
                self.activity_history
                    .newest()
                    .or_nan(|e| format_fraction_as_percent(**e)),
            )
            .lines(self.activity_graph(width))
            .active(args.focused)
            .build(format!("Drive({})", self.supply_name))?;

        Ok(block)
    }

    fn _build_page(&mut self, args: &DetailArg) -> AResult<String> {
        let width = args.rect.width;
        let mut blocks = vec![];

        fn label(history: &Ring<f64>, highest: &f64) -> String {
            let formatted_read_speed = history.newest().or_nan(|e| convert_speed(**e, false));

            let formatted_highest_read_speed = convert_speed(*highest, false);
            format!(
                "{formatted_read_speed} · {} {formatted_highest_read_speed}",
                "Highest:"
            )
        }

        let usage = GroupedLines::builder(width, &self.theme)
            .kv(
                "Drive Activity",
                self.activity_history
                    .newest()
                    .or_nan(|e| format_fraction_as_percent(**e)),
            )
            .lines(self.activity_graph(width - 2))
            .empty_sep()
            .kv(
                "Read Speed",
                label(&self.read_speed_history, &self.read_highest.get()),
            )
            .lines({
                let (history, max) = drive_speed_graph_inputs(
                    DriveSpeedKind::Read,
                    &self.read_speed_history,
                    self.read_highest.get(),
                    &self.write_speed_history,
                    self.write_highest.get(),
                );
                ls_history_graph(
                    width - 2,
                    history,
                    max,
                    0.,
                    3,
                    ratatui::style::Color::Black,
                )
            })
            .empty_sep()
            .kv(
                "Write Speed",
                label(&self.write_speed_history, &self.write_highest.get()),
            )
            .lines({
                let (history, max) = drive_speed_graph_inputs(
                    DriveSpeedKind::Write,
                    &self.read_speed_history,
                    self.read_highest.get(),
                    &self.write_speed_history,
                    self.write_highest.get(),
                );
                ls_history_graph(
                    width - 2,
                    history,
                    max,
                    0.,
                    3,
                    ratatui::style::Color::Black,
                )
            })
            .empty_sep()
            .kv_sep("Total Read", convert_storage(self.read_total.get(), true))
            .kv_sep("Total Write", convert_storage(self.write_total.get(), true))
            .active(args.active)
            .build("Usage")?;

        blocks.push(usage);

        let mut partitions = GroupedLines::builder(width, &self.theme);
        for part in &self.partitions {
            partitions = partitions
                .line(
                    vec![Span::raw(format!(
                        "{} ({}) [{}]  {} / {}",
                        part.device,
                        part.mount_point,
                        part.fs_type,
                        convert_storage(part.used_bytes() as f64, false),
                        convert_storage(part.total_bytes as f64, false)
                    ))]
                    .into(),
                )
                .kv(
                    "Free",
                    format!(
                        "{} ({:.1} %)",
                        convert_storage(part.free_bytes as f64, false),
                        if part.total_bytes == 0 {
                            0.0
                        } else {
                            part.free_bytes as f64 / part.total_bytes as f64 * 100.0
                        }
                    ),
                )
                .line(
                    s_percent_graph(
                        part.used_bytes() as f64,
                        part.total_bytes as f64,
                        width - 2,
                        false,
                    )
                    .into(),
                );
        }

        blocks.push(partitions.active(args.active).build("Partitions")?);

        let props = GroupedLines::builder(width, &self.theme)
            .kv_sep("Sys Path", self.info.sysfs_path.to_str().or_nan_def())
            .kv_sep("Model", self.info.model.or_unk_def())
            .kv_sep("Type", self.info.drive_type.to_string())
            .kv_sep("Device", &self.info.block_device)
            .kv_sep(
                "Capacity",
                self.capacity.or_nan(|e| convert_storage(*e as f64, false)),
            )
            .kv_sep("Writable", self.writiable.or_nan_owned())
            .kv_sep("Removable", self.removeable.or_nan_owned())
            .active(args.active)
            .build("Properties")?;

        blocks.push(props);

        self.viewer_state.update_blocks(blocks);

        Ok(self
            .info
            .model()
            .unwrap_or("Unknown Disk".to_owned())
            .trim()
            .to_owned())
    }

    fn cached_page_state<'b>(&'b mut self) -> StatefulLinesType<'static, 'b> {
        StatefulLinesType::Groups(&mut self.viewer_state)
    }

    fn get_type_name(&self) -> &'static str {
        "Drive"
    }

    fn get_name(&self) -> String {
        self.info.get_name()
    }
}

#[cfg(test)]
mod tests {
    use super::{drive_speed_graph_inputs, DriveSpeedKind, DRIVE_ACTIVITY_GRAPH_MAX};
    use crate::{ring::Ring, tarits::format_fraction_as_percent};

    #[test]
    fn drive_activity_label_and_graph_share_0_1_scale() {
        assert_eq!(DRIVE_ACTIVITY_GRAPH_MAX, 1.0);
        let label = format_fraction_as_percent(0.5);
        assert_eq!(label, "50.0 %");
        assert!(!label.contains("0.5 %"));
    }

    #[test]
    fn drive_read_graph_pairs_read_history_with_read_max() {
        let mut read = Ring::new(8);
        read.insert_at_first(1.0);
        let mut write = Ring::new(8);
        write.insert_at_first(9.0);

        let (history, max) =
            drive_speed_graph_inputs(DriveSpeedKind::Read, &read, 10.0, &write, 99.0);
        assert_eq!(*history.newest().unwrap(), 1.0);
        assert_eq!(max, 10.0);
        assert_ne!(*history.newest().unwrap(), 9.0);
        assert_ne!(max, 99.0);
    }
}
