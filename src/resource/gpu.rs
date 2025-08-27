use std::sync::Arc;

use chin_tools::{AResult, SharedStr};

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
    view::{theme::SharedTheme, BlockArg, DetailArg},
};

use super::{Resource, SensorResultType, SensorRsp};

#[derive(Debug)]
pub struct ResGPU {
    info: Arc<Gpu>,

    // Show
    id: String,
    theme: SharedTheme,

    gpu_data: Option<GpuData>,

    total_usage_history: Ring<f64>,

    video_decode_utilzation_history: Ring<f64>,
    video_encode_utilzation_history: Ring<f64>,

    max_clock_speed: Option<usize>,

    opengl_version: SharedStr,
    vulkan_version: SharedStr,
    pci_express_speed: SharedStr,
    max_pci_express_speed: SharedStr,

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
                    theme: theme.clone(),
                    info: Arc::new(e),
                    total_usage_history: Ring::new(1000),
                    gpu_data: None,
                    viewer_state: StatefulGroupedLines::default(),
                    max_clock_speed: None,
                    opengl_version: "".into(),
                    vulkan_version: "".into(),
                    pci_express_speed: "".into(),
                    max_pci_express_speed: "".into(),
                    video_decode_utilzation_history: Ring::new(1000),
                    video_encode_utilzation_history: Ring::new(1000),
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
        log::info!("update gpu data");
        if let Some(val) = data.usage_fraction {
            self.total_usage_history.insert_at_first(val);
        }
        if let Some(val) = data.decode_fraction {
            self.video_decode_utilzation_history.insert_at_first(val);
        }
        if let Some(val) = data.encode_fraction {
            self.video_encode_utilzation_history.insert_at_first(val);
        }

        self.gpu_data.replace(data.clone());
    }

    fn block(&self, args: &mut BlockArg) -> AResult<GroupedLines<'static>> {
        let width = args.width;
        let title = format!(
            "GPU({})",
            self.info
                .sysfs_path()
                .file_name()
                .map(|e| e.to_str().or_nan_owned())
                .or_unk_def()
        );
        let block = if let Some(gpu_data) = self.gpu_data.as_ref() {
            GroupedLines::builder(width, &self.theme)
                .kv(
                    "UR",
                    gpu_data
                        .usage_fraction
                        .map(|v| format!("{:.1} %", v))
                        .or_nan_def(),
                )
                .lines(ls_history_graph(
                    width,
                    &self.total_usage_history,
                    1.,
                    0.,
                    3,
                    ratatui::style::Color::Red,
                ))
                .active(args.focused)
                .build(title)?
        } else {
            GroupedLines::builder(width, &self.theme).build(title)?
        };

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

    fn _build_page(&mut self, args: &DetailArg) -> AResult<String> {
        let width = args.rect.width;
        let mut blocks = vec![];

        if let Some(gpu_data) = self.gpu_data.as_ref() {
            let usage = GroupedLines::builder(width, &self.theme)
                .kv(
                    "Utilization",
                    gpu_data
                        .usage_fraction
                        .or_nan(|e| format!("{:.1} %", e * 100.)),
                )
                .lines(ls_history_graph(
                    width - 2,
                    &self.total_usage_history,
                    100.,
                    0.,
                    3,
                    ratatui::style::Color::Green,
                ))
                .empty_sep()
                .kv("Clock Speed", {
                    if let (None, None) = (gpu_data.clock_speed, self.max_clock_speed) {
                        gpu_data.clock_speed.or_nan_owned()
                    } else {
                        format!(
                            "{} / {}",
                            gpu_data.clock_speed.or_nan_owned(),
                            self.max_clock_speed.or_nan_owned()
                        )
                    }
                })
                .kv_sep(
                    "VRam Used / Total / Speed",
                    format!(
                        "{} / {} /{}",
                        gpu_data.used_vram.or_nan_owned(),
                        gpu_data.total_vram.or_nan_owned(),
                        gpu_data.vram_speed.or_nan_owned()
                    ),
                )
                .kv("Decode Utilzation", gpu_data.decode_fraction.or_nan_owned())
                .lines(ls_history_graph(
                    width - 2,
                    &self.video_decode_utilzation_history,
                    100.,
                    0.,
                    3,
                    ratatui::style::Color::Green,
                ))
                .kv("Encode Utilzation", gpu_data.encode_fraction.or_nan_owned())
                .lines(ls_history_graph(
                    width - 2,
                    &self.video_encode_utilzation_history,
                    100.,
                    0.,
                    3,
                    ratatui::style::Color::Green,
                ))
                .active(args.active)
                .build("Usage")?;
            blocks.push(usage);
        }

        let props = GroupedLines::builder(width, &self.theme)
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
            .kv("OpenGL Version", self.opengl_version.as_str())
            .kv("Vulkan Version", self.vulkan_version.as_str())
            .kv("PCI Express Speed", self.pci_express_speed.as_str())
            .active(args.active)
            .build("Props")?;
        blocks.push(props);

        self.viewer_state.update_blocks(blocks);

        Ok("".to_string())
    }

    fn handle_navi_event(&mut self, _event: &crate::view::NavigatorEvent) -> bool {
        false
    }
}
