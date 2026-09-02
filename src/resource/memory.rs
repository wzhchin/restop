use chin_tools::AResult;
use itertools::Itertools;

use crate::{
    component::{
        grouped_lines::GroupedLines,
        ls_history_graph,
        stateful_lines::{StatefulGroupedLines, StatefulLinesType},
    },
    ring::{Ring, DEFAULT_HISTORY_LEN},
    sensor::{
        memory::{self, MemoryData, MemoryDevice},
        units::convert_storage,
    },
    tarits::{format_fraction_as_percent, None2NaN, None2NanString},
    view::theme::SharedTheme,
    view::{BlockArg, DetailArg},
};

use super::{map_all_unique, Resource, SensorResultType};

#[derive(Debug)]
pub struct ResMEM {
    info: Vec<MemoryDevice>,

    pub formatted_used_mem: Option<String>,
    pub formatted_available_mem: Option<String>,
    pub formatted_total_mem: Option<String>,
    pub mem_usage_percent: Option<f64>,

    pub usage_history: Ring<f64>,

    pub formatted_used_swap: Option<String>,
    pub formatted_total_swap: Option<String>,
    pub swap_usage_percent: Option<f64>,

    pub swap_usage_history: Ring<f64>,

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
            formatted_available_mem: Default::default(),
            formatted_total_mem: Default::default(),
            mem_usage_percent: Default::default(),
            usage_history: Ring::new(DEFAULT_HISTORY_LEN),
            formatted_used_swap: Default::default(),
            formatted_total_swap: Default::default(),
            swap_usage_percent: Default::default(),
            swap_usage_history: Ring::new(DEFAULT_HISTORY_LEN),
            viewer_state: Default::default(),
        })
    }

    pub fn mem_usage(&self) -> String {
        format_compact_usage(
            self.formatted_used_mem.as_deref(),
            self.mem_usage_percent,
        )
    }

    pub fn swap_usage(&self) -> String {
        format_compact_usage(
            self.formatted_used_swap.as_deref(),
            self.swap_usage_percent,
        )
    }
}

/// Compact used/swap label. Missing or non-finite values are `N/A`, never `NaN`.
pub(crate) fn format_compact_usage(
    formatted_used: Option<&str>,
    usage_fraction: Option<f64>,
) -> String {
    let mut label = String::new();
    match formatted_used {
        Some(v) => label.push_str(v),
        None => label.push_str("N/A"),
    }
    label.push_str(" | ");
    match usage_fraction {
        Some(fraction) if fraction.is_finite() => {
            label.push_str(&format_fraction_as_percent(fraction));
        }
        Some(_) => label.push_str("N/A"),
        None => {}
    }
    label
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
            total_swap,
            free_swap,
        } = *data;

        let used_mem = total_mem.saturating_sub(available_mem);
        let used_swap = total_swap.saturating_sub(free_swap);

        let memory_fraction = used_mem as f64 / total_mem as f64;
        let swap_fraction = if total_swap > 0 {
            used_swap as f64 / total_swap as f64
        } else {
            0.0
        };

        let formatted_used_mem = convert_storage(used_mem as f64, false);
        let formatted_available_mem = convert_storage(available_mem as f64, false);
        let formatted_total_mem = convert_storage(total_mem as f64, false);

        let formatted_used_swap = if total_swap > 0 {
            Some(convert_storage(used_swap as f64, false))
        } else {
            None
        };
        let formatted_total_swap = if total_swap > 0 {
            Some(convert_storage(total_swap as f64, false))
        } else {
            None
        };

        self.mem_usage_percent.replace(memory_fraction);
        self.usage_history.insert_at_first(memory_fraction);
        self.formatted_used_mem.replace(formatted_used_mem);
        self.formatted_available_mem
            .replace(formatted_available_mem);
        self.formatted_total_mem.replace(formatted_total_mem);

        self.swap_usage_percent.replace(swap_fraction);
        self.swap_usage_history.insert_at_first(swap_fraction);
        self.formatted_used_swap = formatted_used_swap;
        self.formatted_total_swap = formatted_total_swap;
    }

    fn block(&self, args: &mut BlockArg) -> AResult<GroupedLines<'static>> {
        let width = args.width;
        let mut builder = GroupedLines::builder(width, &self.theme)
            .kv("Dev", {
                format!(
                    "{}({})",
                    self.formatted_total_mem.or_nan_owned(),
                    self.info.iter().flat_map(|e| &e.r#type).unique().join(" ")
                )
            })
            .kv("Usage", self.mem_usage());

        if self.formatted_total_swap.is_some() && self.formatted_used_swap.is_some() {
            builder = builder.kv("Swap", self.swap_usage());
        }

        let block = builder
            .lines(ls_history_graph(
                width,
                &self.usage_history,
                1.,
                0.,
                3,
                ratatui::style::Color::Black,
            ))
            .active(args.focused)
            .build("Memory")?;

        Ok(block)
    }

    fn _build_page(&mut self, args: &DetailArg) -> AResult<String> {
        let width = args.rect.width;
        let mut block_vec = vec![];

        let usage = GroupedLines::builder(width, &self.theme)
            .kv_sep("Memory", self.mem_usage().as_str())
            .kv_sep("Available", self.formatted_available_mem.or_nan_owned())
            .lines(ls_history_graph(
                width - 2,
                &self.usage_history,
                1.,
                0.,
                3,
                ratatui::style::Color::Black,
            ));

        let usage = if self.formatted_total_swap.is_some() && self.formatted_used_swap.is_some() {
            usage
                .empty_sep()
                .kv_sep("Swap", self.swap_usage().as_str())
                .lines(ls_history_graph(
                    width - 2,
                    &self.swap_usage_history,
                    1.,
                    0.,
                    3,
                    ratatui::style::Color::Black,
                ))
        } else {
            usage
        };

        let usage = usage.active(args.active).build("Usage")?;

        block_vec.push(usage);

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

#[cfg(test)]
mod tests {
    use super::format_compact_usage;

    #[test]
    fn compact_usage_never_renders_nan() {
        for label in [
            format_compact_usage(None, None),
            format_compact_usage(None, Some(f64::NAN)),
            format_compact_usage(Some("1.0 GiB"), Some(f64::NAN)),
            format_compact_usage(Some("1.0 GiB"), Some(f32::NAN as f64)),
        ] {
            assert!(!label.contains("NaN"), "got {label}");
            assert!(label.contains("N/A"), "got {label}");
        }
    }

    #[test]
    fn compact_usage_formats_fraction_as_percent() {
        let label = format_compact_usage(Some("1.0 GiB"), Some(0.5));
        assert!(label.contains("50"));
        assert!(!label.contains("0.5 %"));
        assert!(!label.contains("NaN"));
    }
}
