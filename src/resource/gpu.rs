use std::sync::Arc;

use chin_tools::{AResult, SharedStr};

use crate::{
    component::{
        grouped_lines::GroupedLines,
        ls_history_graph,
        stateful_lines::{StatefulGroupedLines, StatefulLinesType},
    },
    ring::{Ring, DEFAULT_HISTORY_LEN},
    sensor::{
        gpu::{Gpu, GpuData},
        units::{convert_frequency, convert_power, convert_storage, convert_temperature},
    },
    tarits::{format_fraction_as_percent, None2NaN, None2NaNDef, None2NanString},
    view::{theme::SharedTheme, BlockArg, DetailArg},
};

use super::{Resource, SensorResultType, SensorRsp};

/// Utilization/VCN history is stored as 0–1 fractions; graph max matches that unit.
pub(crate) const GPU_USAGE_GRAPH_MAX: f64 = 1.0;

#[derive(Debug)]
pub struct ResGPU {
    info: Arc<Gpu>,

    // Show
    id: String,
    theme: SharedTheme,

    gpu_data: Option<GpuData>,

    total_usage_history: Ring<f64>,

    vcn_usage_history: Ring<f64>,

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

                let (pci_express_speed, max_pci_express_speed) = e
                    .pcie_link()
                    .unwrap_or(("N/A".to_string(), "N/A".to_string()));

                Self {
                    id,
                    theme: theme.clone(),
                    info: Arc::new(e),
                    total_usage_history: Ring::new(DEFAULT_HISTORY_LEN),
                    gpu_data: None,
                    viewer_state: StatefulGroupedLines::default(),
                    pci_express_speed: pci_express_speed.into(),
                    max_pci_express_speed: max_pci_express_speed.into(),
                    vcn_usage_history: Ring::new(DEFAULT_HISTORY_LEN),
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
        if let Some(val) = data.usage_fraction {
            self.total_usage_history.insert_at_first(val);
        }
        if let Some(val) = data.vcn_fraction {
            self.vcn_usage_history.insert_at_first(val);
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
                    format!(
                        "{}  {}",
                        gpu_data
                            .usage_fraction
                            .or_nan(|e| format_fraction_as_percent(*e)),
                        gpu_data
                            .temp
                            .or_nan(|e| convert_temperature(*e)),
                    ),
                )
                .lines(ls_history_graph(
                    width,
                    &self.total_usage_history,
                    GPU_USAGE_GRAPH_MAX,
                    0.,
                    3,
                    ratatui::style::Color::Black,
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
                        .or_nan(|e| format_fraction_as_percent(*e)),
                )
                .lines(ls_history_graph(
                    width - 2,
                    &self.total_usage_history,
                    GPU_USAGE_GRAPH_MAX,
                    0.,
                    3,
                    ratatui::style::Color::Black,
                ))
                .empty_sep()
                .kv(
                    "Temperature",
                    gpu_data.temp.or_nan(|e| convert_temperature(*e)),
                )
                .empty_sep()
                .kv("Clock Speed", {
                    if let (None, None) = (gpu_data.clock_speed, gpu_data.max_clock_speed) {
                        gpu_data.clock_speed.or_nan(|e| convert_frequency(*e))
                    } else {
                        format!(
                            "{} / {}",
                            gpu_data.clock_speed.or_nan(|e| convert_frequency(*e)),
                            gpu_data.max_clock_speed.or_nan(|e| convert_frequency(*e)),
                        )
                    }
                })
                .kv_sep(
                    "VRam Used / Total / Speed",
                    format!(
                        "{} / {} / {}",
                        gpu_data
                            .used_vram
                            .or_nan(|e| convert_storage(*e as f64, false)),
                        gpu_data
                            .total_vram
                            .or_nan(|e| convert_storage(*e as f64, false)),
                        gpu_data.vram_speed.or_nan(|e| convert_frequency(*e)),
                    ),
                )
                .kv("Video Utilzation", gpu_data.vcn_fraction.or_nan(|e| format_fraction_as_percent(*e)))
                .lines(ls_history_graph(
                    width - 2,
                    &self.vcn_usage_history,
                    GPU_USAGE_GRAPH_MAX,
                    0.,
                    3,
                    ratatui::style::Color::Black,
                ))
                .active(args.active)
                .build("Usage")?;
            blocks.push(usage);
        }

        let props = GroupedLines::builder(width, &self.theme)
            .kv_sep("Name", self.info.name().ok().or_unk_def())
            .kv_sep(
                "Manufacturer",
                self.info.get_vendor_name().ok().or_unk_def(),
            )
            .kv_sep("PCI Slot", self.info.pci_slot().to_string())
            .kv_sep("Driver Used", self.info.driver())
            .kv_sep(
                "Power Used / Cap / Max",
                format!(
                    "{} / {} / {}",
                    self.gpu_data
                        .as_ref()
                        .and_then(|d| d.power_usage)
                        .or_nan(|e| convert_power(*e)),
                    self.gpu_data
                        .as_ref()
                        .and_then(|d| d.power_cap)
                        .or_nan(|e| convert_power(*e)),
                    self.info.power_cap_max().ok().or_nan(|e| convert_power(*e)),
                ),
            )
            .kv(
                "PCI Express Speed",
                format!(
                    "{} / {}",
                    self.pci_express_speed.as_str(),
                    self.max_pci_express_speed.as_str(),
                ),
            )
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

#[cfg(test)]
mod tests {
    use super::GPU_USAGE_GRAPH_MAX;
    use crate::tarits::format_fraction_as_percent;

    #[test]
    fn gpu_fraction_label_and_graph_share_0_1_scale() {
        assert_eq!(GPU_USAGE_GRAPH_MAX, 1.0);
        let label = format_fraction_as_percent(0.5);
        assert_eq!(label, "50.0 %");
        assert!(!label.contains("0.5 %"));
        assert!(!format_fraction_as_percent(f64::NAN).contains("NaN"));
    }
}
