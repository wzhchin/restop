use std::{cell::Cell, sync::Arc, time::SystemTime};

use chin_tools::AResult;
use ratatui::layout::Rect;

use crate::{
    component::{
        grouped_lines::GroupedLines,
        ls_history_graph,
        stateful_lines::{StatefulGroupedLines, StatefulLinesType},
    },
    ring::Ring,
    sensor::{
        network::{NetworkData, NetworkInterface},
        units::{convert_speed, convert_storage},
        Sensor,
    },
    tarits::{None2NaN, None2NaNDef, None2NanString},
    view::theme::SharedTheme,
    view::{OverviewArg, PageArg},
};

use super::{Resource, SensorResultType, SensorRsp};

#[derive(Debug)]
pub struct ResNetwork {
    info: Arc<NetworkInterface>,

    last_timestamp: Option<SystemTime>,

    old_received_bytes: Option<usize>,
    old_sent_bytes: Option<usize>,

    highest_received_speed: Cell<f64>,
    highest_sent_speed: Cell<f64>,

    received_speed: Option<f64>,
    sent_speed: Option<f64>,

    // Show
    theme: SharedTheme,
    sendhistory: Ring<f64>,
    receive_history: Ring<f64>,

    viewer_state: StatefulGroupedLines<'static>,
}

impl ResNetwork {
    pub fn new(theme: SharedTheme) -> AResult<Vec<Self>> {
        let network_paths = NetworkInterface::get_sysfs_paths().unwrap_or_default();

        let rns = network_paths
            .iter()
            .map(|path| ResNetwork {
                info: Arc::new(NetworkInterface::from_sysfs(path)),
                theme: theme.clone(),
                old_received_bytes: None,
                old_sent_bytes: None,
                last_timestamp: None,
                highest_received_speed: Default::default(),
                highest_sent_speed: Default::default(),
                received_speed: None,
                sent_speed: None,
                sendhistory: Ring::new(1000),
                receive_history: Ring::new(1000),
                viewer_state: Default::default(),
            })
            .collect();

        Ok(rns)
    }

    fn interface(&self) -> String {
        self.info.interface_name.to_str().or_unk(|e| e.to_string())
    }
}

impl Resource for ResNetwork {
    type Req = Arc<NetworkInterface>;

    type Rsp = NetworkData;

    fn get_id(&self) -> &str {
        self.info.hw_address.as_ref().map_or("", |e| e)
    }

    fn get_req(&self) -> Self::Req {
        self.info.clone()
    }

    fn do_sensor(req: Self::Req) -> AResult<SensorResultType> {
        let data = NetworkData::new(&req);
        Ok(SensorResultType::SyncResult(
            SensorRsp::Network(data).into(),
        ))
    }

    fn update_data(&mut self, data: &Self::Rsp) {
        let NetworkData {
            received_bytes,
            sent_bytes,
            is_virtual: _,
            display_name: _,
            hw_address: _,
            sysfs_path: _,
        } = data;

        if let (Some(old_time), Some(old_received_bytes), Some(old_sent_bytes)) = (
            self.last_timestamp,
            self.old_received_bytes,
            self.old_sent_bytes,
        ) {
            let time_passed = SystemTime::now()
                .duration_since(old_time)
                .map_or(1.0f64, |timestamp| timestamp.as_secs_f64());

            let received_delta = if let (Ok(received_bytes),) = (received_bytes,) {
                Some(received_bytes.saturating_sub(old_received_bytes) as f64 / time_passed)
            } else {
                None
            };

            let sent_delta = if let (Ok(sent_bytes),) = (sent_bytes,) {
                Some(sent_bytes.saturating_sub(old_sent_bytes) as f64 / time_passed)
            } else {
                None
            };

            if let Some(ok) = sent_delta.as_ref() {
                self.sendhistory.insert_at_first(*ok);
            }

            if let Some(ok) = received_delta.as_ref() {
                self.receive_history.insert_at_first(*ok);
            }

            self.sent_speed = sent_delta;
            self.received_speed = received_delta;

            if self
                .sent_speed
                .is_some_and(|e| e > self.highest_sent_speed.get())
            {
                if let Some(e) = sent_delta.as_ref() {
                    self.highest_sent_speed.set(*e)
                }
            }

            if self
                .received_speed
                .is_some_and(|e| e > self.highest_received_speed.get())
            {
                if let Some(e) = received_delta.as_ref() {
                    self.highest_received_speed.set(*e)
                }
            }
        }

        self.last_timestamp.replace(SystemTime::now());
        self.old_received_bytes = received_bytes.as_ref().map(|e| *e).ok();
        self.old_sent_bytes = sent_bytes.as_ref().map(|e| *e).ok();
    }

    fn overview_content(&self, args: &mut OverviewArg) -> AResult<GroupedLines<'static>> {
        let width = args.width;
        let block = GroupedLines::builder(width, &self.theme)
            .multi_kv_single_line(vec![
                (
                    "R",
                    self.received_speed
                        .as_ref()
                        .or_nan(|e| convert_storage(**e, false)),
                ),
                (
                    "S",
                    self.sent_speed
                        .as_ref()
                        .or_nan(|e| convert_storage(**e, false)),
                ),
            ])
            .lines(ls_history_graph(
                width,
                &self.sendhistory,
                self.highest_sent_speed.get(),
                0.,
                3,
                ratatui::style::Color::Yellow,
            ))
            .lines(ls_history_graph(
                width,
                &self.receive_history,
                self.highest_received_speed.get(),
                0.,
                3,
                ratatui::style::Color::Blue,
            ))
            .active(args.focused)
            .build(format!(
                "{}({})",
                self.info.interface_type.short_type(),
                self.info.interface_name.to_str().or_unk_def()
            ))?;

        Ok(block)
    }

    fn _build_page(&mut self, args: &PageArg) -> AResult<String> {
        let mut blocks = vec![];
        let Rect {
            width,
            height: _,
            x: _,
            y: _,
        } = args.rect;

        fn label(history: &Ring<f64>, highest: &f64) -> String {
            let formatted_read_speed = history.newest().or_nan(|e| convert_speed(**e, false));

            let formatted_highest_read_speed = convert_speed(*highest, false);
            format!(
                "{formatted_read_speed} · {} {formatted_highest_read_speed}",
                "Highest:"
            )
        }

        let usage = GroupedLines::builder(width, &self.theme)
            .kv_sep(
                "Receiving",
                label(&self.receive_history, &self.highest_received_speed.get()),
            )
            .lines(ls_history_graph(
                width - 2,
                &self.receive_history,
                self.highest_received_speed.get(),
                0.,
                3,
                ratatui::style::Color::Blue,
            ))
            .kv_sep(
                "Sending",
                label(&self.sendhistory, &self.highest_sent_speed.get()),
            )
            .lines(ls_history_graph(
                width - 2,
                &self.sendhistory,
                self.highest_sent_speed.get(),
                0.,
                3,
                ratatui::style::Color::Yellow,
            ))
            .kv_sep(
                "Total Received",
                self.old_received_bytes
                    .or_nan(|e| convert_storage(*e as f64, false))
                    .as_str(),
            )
            .kv_sep(
                "Total Sent",
                self.old_sent_bytes
                    .or_nan(|e| convert_storage(*e as f64, false))
                    .as_str(),
            )
            .active(args.active)
            .build("Usage")?;
        blocks.push(usage);

        let props = GroupedLines::builder(width, &self.theme)
            .kv_sep("Sys Path", self.info.sysfs_path.to_str().or_nan_def())
            .kv_sep("Conection Type", self.info.interface_type.to_string())
            .kv_sep("Manufacturer", self.info.vendor.or_unk_def())
            .kv_sep("Driver Used", self.info.driver_name.or_unk_def())
            .kv_sep("Interface", self.interface().as_str())
            .kv_sep("Hardware Address", self.info.hw_address.or_nan_def())
            .active(args.active)
            .build("Properties")?;
        blocks.push(props);

        self.viewer_state.update_blocks(blocks);

        Ok(self.info.pid_name.or_nan_owned())
    }

    fn cached_page_state<'b>(&'b mut self) -> StatefulLinesType<'static, 'b> {
        StatefulLinesType::Groups(&mut self.viewer_state)
    }

    fn get_type_name(&self) -> &'static str {
        "Network"
    }

    fn get_name(&self) -> String {
        self.info.get_name()
    }
}
