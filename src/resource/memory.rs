use chin_tools::AResult;
use itertools::Itertools;

use crate::{
    component::{
        grouped_lines::GroupedLines,
        ls_history_graph,
        stateful_lines::{StatefulGroupedLines, StatefulLinesType},
    },
    ring::Ring,
    sensor::{
        memory::{self, MemoryData, MemoryDevice},
        units::convert_storage,
    },
    tarits::{None2NaN, None2NanString},
    view::theme::SharedTheme,
    view::{OverviewArg, PageArg},
};

use super::{map_all_unique, Resource, SensorResultType};

#[derive(Debug)]
pub struct ResMEM {
    info: Vec<MemoryDevice>,

    pub formatted_used_mem: Option<String>,
    pub formatted_total_mem: Option<String>,
    pub mem_usage_percent: Option<f64>,

    pub usage_history: Ring<f64>,

    // Show
    theme: SharedTheme,

    viewer_state: StatefulGroupedLines<'static>,
}

impl ResMEM {
    pub fn new(theme: SharedTheme) -> AResult<Self> {
        let meminfo = memory::get_memory_devices()?;

        Ok(Self {
            info: meminfo,

            theme,
            formatted_used_mem: Default::default(),
            formatted_total_mem: Default::default(),
            mem_usage_percent: Default::default(),
            usage_history: Ring::new(1000),
            viewer_state: Default::default(),
        })
    }

    pub fn mem_usage(&self) -> String {
        let mut label = String::new();
        match self.formatted_used_mem.as_ref() {
            Some(v) => label.push_str(v.as_str()),
            None => label.push_str("NaN"),
        };

        label.push_str(" | ");

        if let Some(percent) = self.mem_usage_percent.as_ref() {
            label.push_str(format!("{:.1} %", percent * 100.).as_str());
        }
        label
    }
}

impl Resource for ResMEM {
    type Req = ();

    type Rsp = MemoryData;

    fn get_id(&self) -> &str {
        "MEM"
    }

    fn get_req(&self) -> Self::Req {}

    fn do_sensor(req: Self::Req) -> AResult<SensorResultType> {
        let data = MemoryData::fetch(req)?;

        Ok(SensorResultType::SyncResult(
            super::SensorRsp::Memory(data).into(),
        ))
    }

    fn update_data(&mut self, data: &Self::Rsp) {
        let MemoryData {
            total_mem,
            available_mem,
            total_swap: _,
            free_swap: _,
        } = *data;

        let used_mem = total_mem.saturating_sub(available_mem);
        // let used_swap = total_swap.saturating_sub(free_swap);

        let memory_fraction = used_mem as f64 / total_mem as f64;
        // let swap_fraction = (used_swap as f64 / total_swap as f64).nan_default(0.0);

        let formatted_used_mem = convert_storage(used_mem as f64, false);
        let formatted_total_mem = convert_storage(total_mem as f64, false);

        self.mem_usage_percent.replace(memory_fraction);
        self.usage_history.insert_at_first(memory_fraction);
        self.formatted_used_mem.replace(formatted_used_mem);
        self.formatted_total_mem.replace(formatted_total_mem);
    }

    fn overview_content(&self, args: &mut OverviewArg) -> AResult<GroupedLines<'static>> {
        let width = args.width;
        let block = GroupedLines::builder(width, &self.theme)
            .kv("Dev", {
                format!(
                    "{}({})",
                    self.formatted_total_mem.or_nan_owned(),
                    self.info.iter().flat_map(|e| &e.r#type).unique().join(" ")
                )
            })
            .kv("Usage", self.mem_usage())
            .lines(ls_history_graph(
                width,
                &self.usage_history,
                1.,
                0.,
                3,
                ratatui::style::Color::Magenta,
            ))
            .active(args.focused)
            .build("Memory")?;

        Ok(block)
    }

    fn _build_page(&mut self, args: &PageArg) -> AResult<String> {
        let width = args.rect.width;
        let mut block_vec = vec![];

        let usage = GroupedLines::builder(width, &self.theme)
            .kv_sep("Memory", self.mem_usage().as_str())
            .lines(ls_history_graph(
                width - 2,
                &self.usage_history,
                1.,
                0.,
                3,
                ratatui::style::Color::Magenta,
            ))
            .active(args.active)
            .build("Usage")?;

        let props = GroupedLines::builder(width, &self.theme)
            .kv_sep("Slot Usage", self.info.len().to_string().as_str())
            .kv_sep("Speed", {
                map_all_unique(self.info.iter(), |e| e.speed_mts.or_nan(|e| e.to_string()))
                    .join(" ")
                    .as_str()
            })
            .kv_sep("Form Factor", {
                map_all_unique(self.info.iter(), |e| e.form_factor.or_nan_owned())
                    .join(" ")
                    .as_str()
            })
            .kv_sep("Type", {
                map_all_unique(self.info.iter(), |e| e.r#type.or_nan_owned())
                    .join(" ")
                    .as_str()
            })
            .kv_sep("Type Detail", {
                map_all_unique(self.info.iter(), |e| e.type_detail.or_nan_owned())
                    .join(" ")
                    .as_str()
            })
            .active(args.active)
            .build("Properties")?;
        block_vec.push(usage);
        block_vec.push(props);

        self.viewer_state.update_blocks(block_vec);

        Ok("".to_string())
    }

    fn cached_page_state<'b>(&'b mut self) -> StatefulLinesType<'static, 'b> {
        StatefulLinesType::Groups(&mut self.viewer_state)
    }

    fn get_type_name(&self) -> &'static str {
        "Memory"
    }

    fn get_name(&self) -> String {
        "".to_string()
    }
}
