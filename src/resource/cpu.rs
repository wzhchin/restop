use std::cell::{Cell, RefCell};

use anyhow::Context;
use chin_tools::AResult;
use ratatui::text::{Line, Span};

use crate::{
    component::{
        grouped_lines::GroupedLines,
        ls_history_graph, s_history_graph,
        stateful_lines::{StatefulGroupedLines, StatefulLinesType},
        PaddingH,
    },
    ring::{Ring, DEFAULT_HISTORY_LEN, SHORT_HISTORY_LEN},
    sensor::{
        cpu::{cpu_info, CpuData, CpuInfo},
        units::{convert_frequency, convert_temperature},
    },
    tarits::{format_percent_label, None2NaN, None2NaNDef},
    view::{theme::SharedTheme, BlockArg, DetailArg},
};

use super::{Resource, SensorResultType, SensorRsp};

/// Consecutive-sample CPU usage on a 0–100 scale.
/// Previous `(0, 0)` is "no sample yet". A zero counter delta is 0, never a
/// since-boot lifetime average and never a non-finite value.
pub(crate) fn cpu_delta_percent(new: (u64, u64), old: (u64, u64)) -> f64 {
    if old == (0, 0) {
        return 0.0;
    }
    let idle_delta = new.0.saturating_sub(old.0);
    let sum_delta = new.1.saturating_sub(old.1);
    if sum_delta == 0 {
        return 0.0;
    }
    let work_time = sum_delta.saturating_sub(idle_delta);
    let fraction = (work_time as f64) / (sum_delta as f64);
    if !fraction.is_finite() {
        0.0
    } else {
        (fraction * 100.0).clamp(0.0, 100.0)
    }
}

#[derive(Debug)]
pub struct ResCPU {
    info: CpuInfo,

    old_total_usage: Cell<(u64, u64)>,
    old_thread_usages: RefCell<Vec<(u64, u64)>>,
    logical_cpus_amount: Cell<usize>,

    // Show
    theme: SharedTheme,

    total_history: Ring<f64>,
    thread_history: Vec<Ring<f64>>,

    frequency: Option<u64>,
    tempurature: Option<f32>,
    viewer_state: StatefulGroupedLines<'static>,
}

impl ResCPU {
    pub fn new(theme: SharedTheme) -> AResult<Self> {
        let cpu_info = cpu_info()?;
        let logic_size = cpu_info
            .logical_cpus
            .context("Unable to get logical core size.")?;

        Ok(Self {
            info: cpu_info,
            old_total_usage: Cell::default(),
            old_thread_usages: RefCell::default(),
            logical_cpus_amount: Cell::new(logic_size),
            total_history: Ring::new(DEFAULT_HISTORY_LEN).name("CCPU"),
            theme,
            tempurature: None,
            thread_history: vec![],
            frequency: None,
            viewer_state: Default::default(),
        })
    }
}

impl Resource for ResCPU {
    type Req = usize;

    type Rsp = CpuData;

    fn get_id(&self) -> &str {
        "CPU"
    }

    fn get_req(&self) -> Self::Req {
        self.logical_cpus_amount.get()
    }

    fn do_sensor(req: Self::Req) -> AResult<SensorResultType> {
        let data = CpuData::fetch(req)?;
        Ok(SensorResultType::SyncResult(SensorRsp::CPU(data).into()))
    }

    fn update_data(&mut self, data: &Self::Rsp) {
        let CpuData {
            new_total_usage,
            new_thread_usages,
            temperature,
            frequency,
        } = data;

        if self.thread_history.len() != new_thread_usages.len() {
            self.thread_history = new_thread_usages
                .iter()
                .map(|_| Ring::new(SHORT_HISTORY_LEN))
                .collect();
        }

        let total_percentage = cpu_delta_percent(*new_total_usage, self.old_total_usage.get());
        {
            let old = self.old_thread_usages.borrow();
            for (index, old_usage) in old.iter().enumerate() {
                if let (Some(new), Some(history)) = (
                    new_thread_usages.get(index),
                    self.thread_history.get_mut(index),
                ) {
                    history.insert_at_first(cpu_delta_percent(*new, *old_usage));
                }
            }
        }

        self.total_history.insert_at_first(total_percentage);

        self.old_total_usage.set(*new_total_usage);
        *self.old_thread_usages.borrow_mut() = new_thread_usages.clone();
        self.tempurature = *temperature;
        self.frequency = *frequency;
    }

    fn block(&self, args: &mut BlockArg) -> AResult<GroupedLines<'static>> {
        let width = args.width;
        let block = GroupedLines::builder(width, &self.theme)
            .kv(
                "UR",
                format!(
                    "{}  {}",
                    self.total_history
                        .newest()
                        .or_nan(|e| format_percent_label(**e)),
                    self.tempurature.or_nan(|e| convert_temperature(*e as f64))
                ),
            )
            .kv(
                "Freq",
                self.frequency
                    .or_nan(|frequency| convert_frequency(*frequency as f64)),
            )
            .lines(ls_history_graph(
                width,
                &self.total_history,
                100.,
                0.,
                3,
                ratatui::style::Color::Black,
            ))
            .active(args.focused)
            .build("CPU")?;

        Ok(block)
    }

    fn _build_page(&mut self, args: &DetailArg) -> AResult<String> {
        let width = args.rect.width;
        let mut result = vec![];
        let info = &self.info;

        let graphs: Vec<Line<'static>> = self
            .thread_history
            .iter()
            .enumerate()
            .map(|(id, ring)| {
                (
                    id,
                    s_history_graph(
                        width.saturating_sub(15),
                        ring,
                        100.,
                        0.,
                        1,
                        ratatui::style::Color::Black,
                    )
                    .padding(),
                    ring.newest(),
                )
            })
            .map(|(index, mut spans, newest)| {
                Line::from({
                    spans.insert(0, Span::raw(format!("{:02}", index)));
                    spans.push(Span::raw(newest.or_nan(|e| {
                        if e.is_finite() {
                            format!("{:<.0}%", **e)
                        } else {
                            "N/A".to_owned()
                        }
                    })));
                    spans
                })
            })
            .collect();

        let graphs = GroupedLines::builder(width, &self.theme)
            .lines(graphs)
            .active(args.active)
            .build("Usage")?;
        result.push(graphs);

        let sensors = GroupedLines::builder(width, &self.theme)
            .kv_sep(
                "Temperature",
                self.tempurature.or_nan(|e| convert_temperature(*e as f64)),
            )
            .kv_sep(
                "Sampled Frequency",
                self.frequency
                    .or_nan(|frequency| convert_frequency(*frequency as f64)),
            )
            .active(args.active);

        result.push(sensors.build("Sensors")?);

        let properties = GroupedLines::builder(width, &self.theme)
            .kv_sep("Model", info.model_name.or_unk_def())
            .kv_sep(
                "Max Frequency",
                self.info
                    .max_speed
                    .clone()
                    .or_nan(|e| convert_frequency(*e)),
            )
            .kv_sep(
                "Logical Cores",
                info.logical_cpus.or_nan(|e| format!("{}", e)),
            )
            .kv_sep(
                "Physical Cores",
                info.physical_cpus.or_nan(|e| format!("{}", e)),
            )
            .kv_sep("Sockets", info.sockets.or_nan(|e| format!("{}", e)))
            .kv_sep(
                "Virtualization",
                info.virtualization.or_nan(|e| e.to_owned()),
            )
            .kv_sep("Architecture", info.architecture.or_nan(|e| e.to_owned()))
            .active(args.active)
            .build("Properties")?;

        result.push(properties);

        self.viewer_state.update_blocks(result);

        Ok(self
            .info
            .model_name
            .as_ref()
            .map_or("CPU".to_string(), |e| e.to_owned()))
    }

    fn cached_page_state<'b>(&'b mut self) -> StatefulLinesType<'static, 'b> {
        StatefulLinesType::Groups(&mut self.viewer_state)
    }

    fn get_type_name(&self) -> &'static str {
        "CPU"
    }

    fn get_name(&self) -> String {
        "".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::cpu_delta_percent;

    #[test]
    fn missing_previous_or_equal_samples_are_zero_not_lifetime() {
        // Previous (0, 0) would otherwise be a since-boot average of 50%.
        assert_eq!(cpu_delta_percent((50, 100), (0, 0)), 0.0);
        assert_eq!(cpu_delta_percent((100, 200), (100, 200)), 0.0);
        assert_eq!(cpu_delta_percent((0, 0), (0, 0)), 0.0);
    }

    #[test]
    fn consecutive_delta_is_percent_on_0_to_100() {
        // idle 50 of 100 ticks busy → 50%.
        assert_eq!(cpu_delta_percent((150, 300), (100, 200)), 50.0);
        assert!(cpu_delta_percent((150, 300), (100, 200)).is_finite());
    }
}
