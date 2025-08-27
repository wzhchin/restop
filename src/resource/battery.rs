use std::{path::PathBuf, sync::Arc};

use chin_tools::AResult;
use process_data::ReuseReader;
use ratatui::layout::Rect;

use crate::{
    component::{
        grouped_lines::GroupedLines,
        s_percent_graph,
        stateful_lines::{StatefulGroupedLines, StatefulLinesType},
    },
    sensor::{
        battery::{Battery, BatteryData},
        units::convert_energy,
        Sensor,
    },
    tarits::{None2NaN, None2NaNDef},
    view::theme::SharedTheme,
    view::{BlockArg, DetailArg},
};

use super::{Resource, SensorResultType, SensorRsp};

#[derive(Debug)]
pub struct ResBattery {
    info: Battery,
    path: Arc<PathBuf>,
    data: Option<Arc<BatteryData>>,
    theme: SharedTheme,
    viewer_state: StatefulGroupedLines<'static>,
}

impl ResBattery {
    pub fn new(theme: SharedTheme) -> AResult<Vec<Self>> {
        let paths = Battery::get_sysfs_paths(&mut ReuseReader::new())?;
        let bs = paths
            .into_iter()
            .map(|path| ResBattery {
                info: Battery::from_sysfs(&path),
                data: None,
                theme: theme.clone(),
                path: Arc::new(path),
                viewer_state: Default::default(),
            })
            .collect();

        Ok(bs)
    }
}

impl Resource for ResBattery {
    type Req = Arc<PathBuf>;

    type Rsp = Arc<BatteryData>;

    fn do_sensor(req: Self::Req) -> AResult<SensorResultType> {
        let data = BatteryData::new(req.as_ref());
        Ok(SensorResultType::SyncResult(
            SensorRsp::Battery(Arc::new(data)).into(),
        ))
    }

    fn get_id(&self) -> &str {
        self.info
            .sysfs_path
            .as_path()
            .to_str()
            .map_or("bat=====", |e| e)
    }

    fn get_req(&self) -> Self::Req {
        self.path.clone()
    }

    fn block(&self, args: &mut BlockArg) -> AResult<GroupedLines<'static>> {
        let width = args.width;
        let block = GroupedLines::builder(width, &self.theme)
            .line(
                s_percent_graph(
                    self.data
                        .as_ref()
                        .map_or(0., |e| e.charge.as_ref().map_or(0., |e| *e)),
                    1.,
                    width.saturating_sub(2),
                    true,
                )
                .into(),
            )
            .active(args.focused)
            .build(format!("Battery({})", &self.info.supply_name))?;

        Ok(block)
    }

    fn _build_page(&mut self, args: &DetailArg) -> AResult<String> {
        let Rect {
            width,
            height: _,
            x: _,
            y: _,
        } = args.rect;
        let info = &self.info;
        let mut blocks = vec![];
        let usage = GroupedLines::builder(width, &self.theme)
            .line(
                s_percent_graph(
                    self.data
                        .as_ref()
                        .map_or(0., |e| e.charge.as_ref().map_or(0., |e| *e)),
                    1.,
                    width - 2,
                    true,
                )
                .into(),
            )
            .active(args.active)
            .build("Usage")?;

        blocks.push(usage);

        let properties = GroupedLines::builder(width, &self.theme)
            .kv_sep("Sys Path", info.sysfs_path.to_str().or_nan_def())
            .kv_sep(
                "Battery Health",
                info.health().ok().or_nan(|e| format!("{:.1} %", e * 100.)),
            )
            .kv_sep(
                "Design Capacity",
                info.design_capacity.or_nan(|e| convert_energy(*e, false)),
            )
            .kv_sep(
                "Charge Cycles",
                info.charge_cycles().ok().or_nan(|e| format!("{}", e)),
            )
            .kv_sep("Technology", info.technology.to_string())
            .kv_sep("Manufacturer", info.manufacturer.or_unk_def())
            .kv_sep("Model Name", info.model_name.or_unk_def())
            .kv_sep("Device", self.info.supply_name.clone())
            .active(args.active)
            .build("Properties")?;

        blocks.push(properties);

        self.viewer_state.update_blocks(blocks);

        Ok(info.model_name.or_unk_def().to_owned())
    }

    fn update_data(&mut self, data: &Self::Rsp) {
        self.data.replace(data.clone());
    }

    fn cached_page_state<'b>(&'b mut self) -> StatefulLinesType<'static, 'b> {
        StatefulLinesType::Groups(&mut self.viewer_state)
    }

    fn get_type_name(&self) -> &'static str {
        self.info.get_type_name()
    }

    fn get_name(&self) -> String {
        self.info.get_name()
    }
}
