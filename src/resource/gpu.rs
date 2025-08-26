use std::sync::Arc;

use chin_tools::AResult;

use crate::{
    component::{
        grouped_lines::GroupedLines,
        ls_history_graph,
        stateful_lines::{StatefulGroupedLines, StatefulLinesType},
    },
    ring::Ring,
    sensor::{
        gpu::{Gpu, GpuData},
        units::convert_power,
    },
    tarits::{None2NaN, None2NaNDef, None2NanString},
    view::{theme::SharedTheme, OverviewArg, PageArg},
};

use super::{Resource, SensorResultType, SensorRsp};

#[derive(Debug)]
pub struct ResGPU {
    info: Arc<Gpu>,

    // Show
    id: String,
    theme: SharedTheme,

    gpu_data: Option<GpuData>,

    total_usage: Option<f64>,
    history: Ring<f64>,

    viewer_state: StatefulGroupedLines<'static>,
}

impl ResGPU {
    pub fn new(theme: SharedTheme) -> AResult<Vec<Self>> {
        let gpu_infos = Gpu::get_gpus()?;

        Ok(gpu_infos
            .into_iter()
            .map(|e| {
                let id = e.pci_slot().to_string();
                Self {
                    id,
                    total_usage: None,
                    theme: theme.clone(),
                    info: Arc::new(e),
                    history: Ring::new(1000),
                    gpu_data: None,
                    viewer_state: StatefulGroupedLines::default(),
                }
            })
            .collect())
    }
}

impl Resource for ResGPU {
    type Req = Arc<Gpu>;

    type Rsp = GpuData;

    fn get_id(&self) -> &str {
        &self.id
    }

    fn get_req(&self) -> Self::Req {
        self.info.clone()
    }

    fn do_sensor(req: Self::Req) -> AResult<SensorResultType> {
        let data = GpuData::new(&req);
        Ok(SensorResultType::SyncResult(SensorRsp::GPU(data).into()))
    }

    fn update_data(&mut self, data: &Self::Rsp) {
        let uf = data.usage_fraction;

        self.history.insert_at_first(uf);
        self.total_usage.replace(uf);

        self.gpu_data.replace(data.clone());
    }

    fn overview_content(&self, args: &mut OverviewArg) -> AResult<GroupedLines<'static>> {
        let width = args.width;
        let block = GroupedLines::builder(width, &self.theme)
            .kv("UR", self.total_usage.or_nan(|e| format!("{:.1} %", e)))
            .lines(ls_history_graph(
                width,
                &self.history,
                1.,
                0.,
                3,
                ratatui::style::Color::Red,
            ))
            .active(args.focused)
            .build(format!(
                "GPU({})",
                self.info
                    .sysfs_path()
                    .file_name()
                    .map(|e| e.to_str().or_nan_owned())
                    .or_unk_def()
            ))?;

        Ok(block)
    }

    fn cached_page_state<'b>(&'b mut self) -> StatefulLinesType<'static, 'b> {
        StatefulLinesType::Groups(&mut self.viewer_state)
    }

    fn get_type_name(&self) -> &'static str {
        "GPU"
    }

    fn get_name(&self) -> String {
        "GPU".to_string()
    }

    fn _build_page(&mut self, args: &PageArg) -> AResult<String> {
        let width = args.rect.width;
        let mut blocks = vec![];

        let usage = GroupedLines::builder(width, &self.theme)
            .kv_sep(
                "Manufacturer",
                self.info.get_vendor_name().ok().or_unk_def(),
            )
            .kv_sep("PCI Slot", self.info.pci_slot().to_string())
            .kv_sep("Driver Used", self.info.driver())
            .kv_sep(
                "Max Power Cap",
                self.info.power_cap_max().ok().or_nan(|e| convert_power(*e)),
            )
            .active(args.active)
            .build("Usage")?;
        blocks.push(usage);

        self.viewer_state.update_blocks(blocks);

        Ok("".to_string())
    }

    fn handle_navi_event(&mut self, _event: &crate::view::NavigatorEvent) -> bool {
        false
    }
}
