pub mod battery;
pub mod cpu;
pub mod drive;
pub mod gpu;
pub mod memory;
pub mod network;
pub mod process;

use std::{
    path::PathBuf,
    sync::Arc,
    thread::{self},
};

use battery::ResBattery;
use chin_tools::AResult;
use cpu::ResCPU;
use drive::{ResDrive, ResDriveRsp};
use flume::Sender;
use gpu::ResGPU;
use itertools::Itertools;
use memory::ResMEM;
use network::ResNetwork;
use process::{ProcessRsp, ResProcess};
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    Frame,
};

use crate::{
    app::ResourceEvent,
    component::{grouped_lines::GroupedLines, stateful_lines::StatefulLinesType},
    sensor::{
        battery::BatteryData,
        cpu::CpuData,
        gpu::{Gpu, GpuData},
        memory::MemoryData,
        network::{NetworkData, NetworkInterface},
    },
    view::{BlockArg, DetailArg, NavigatorEvent},
};

pub trait Resource {
    type Req;
    type Rsp;

    fn get_type_name(&self) -> &'static str;

    fn get_name(&self) -> String;

    fn get_id(&self) -> &str;

    fn get_req(&self) -> Self::Req;

    fn do_sensor(req: Self::Req) -> AResult<SensorResultType>;

    fn update_data(&mut self, data: &Self::Rsp);

    fn block(&self, args: &mut BlockArg) -> AResult<GroupedLines<'static>>;

    fn _build_page(&mut self, _args: &DetailArg) -> AResult<String> {
        Ok("Not Supported".to_string())
    }

    fn cached_page_state<'b>(&'b mut self) -> StatefulLinesType<'static, 'b>;

    fn handle_navi_event(&mut self, _event: &NavigatorEvent) -> bool {
        false
    }

    fn render_detail(&mut self, frame: &mut Frame, args: &DetailArg, max_width: u16) {
        let rect = if max_width > 0 && args.rect.width > max_width {
            let side = (args.rect.width - max_width) / 2;
            Rect {
                x: args.rect.x.saturating_add(side),
                width: args.rect.width.saturating_sub(side * 2),
                ..args.rect
            }
        } else {
            args.rect
        };

        let args = DetailArg {
            rect,
            ..args.clone()
        };

        match self._build_page(&args) {
            Ok(_) => {}
            Err(err) => {
                log::error!("unable to render_page: {}", err);
                return;
            }
        }

        let lines = self.cached_page_state();

        match lines {
            StatefulLinesType::Groups(ls) => {
                ls.render(frame, rect, args.active);
            }
            StatefulLinesType::Lines(vls) => vls.render(frame, rect),
        }
    }
}

#[derive(Debug)]
pub enum SensorRsp {
    CPU(CpuData),
    Memory(MemoryData),
    GPU(AResult<GpuData>),
    Drive(ResDriveRsp),
    Network(NetworkData),
    Battery(Arc<BatteryData>),
    Process(ProcessRsp),
}

pub enum SensorResultType {
    SyncResult(Arc<SensorRsp>),
    AsyncResult,
}

impl SensorRsp {
    fn get_id(&self) -> &str {
        match self {
            SensorRsp::CPU(_) => "CPU",
            SensorRsp::Memory(_) => "MEM",
            SensorRsp::GPU(res) => res.as_ref().map_or("GPU", |e| &e.id),
            SensorRsp::Drive(rsp) => rsp
                .data
                .inner
                .sysfs_path
                .as_path()
                .to_str()
                .unwrap_or("drive"),
            SensorRsp::Network(data) => data.sysfs_path.as_str(),
            SensorRsp::Battery(data) => data
                .inner
                .sysfs_path
                .as_path()
                .to_str()
                .unwrap_or("battery"),
            SensorRsp::Process(_) => "process",
        }
    }
}

pub enum SensorReq {
    CPU(usize),
    Memory(()),
    GPU(Arc<Gpu>),
    Drive(Arc<PathBuf>),
    Network(Arc<NetworkInterface>),
    Battery(Arc<PathBuf>),
    Process(()),
}

#[derive(Debug)]
pub enum ResourceType {
    CPU(ResCPU),
    Memory(ResMEM),
    GPU(ResGPU),
    Drive(ResDrive),
    Network(ResNetwork),
    Battery(ResBattery),
    Process(ResProcess),
}

impl ResourceType {
    pub fn is_process(&self) -> bool {
        matches!(self, ResourceType::Process(_))
    }

    pub fn fetch_data(&self, tx: &Sender<SensorReq>) {
        let rsp = match self {
            ResourceType::CPU(rt) => SensorReq::CPU(rt.get_req()),
            ResourceType::Memory(rt) => {
                let _: () = rt.get_req();
                SensorReq::Memory(())
            }
            ResourceType::GPU(rt) => SensorReq::GPU(rt.get_req()),
            ResourceType::Drive(rt) => SensorReq::Drive(rt.get_req()),
            ResourceType::Network(rt) => SensorReq::Network(rt.get_req()),
            ResourceType::Battery(rt) => SensorReq::Battery(rt.get_req()),
            ResourceType::Process(rt) => {
                let _: () = rt.get_req();
                SensorReq::Process(())
            }
        };
        // Drop if the worker is busy rather than growing an unbounded queue.
        let _ = tx.try_send(rsp);
    }

    pub fn get_id(&self) -> &str {
        match self {
            ResourceType::CPU(d) => d.get_id(),
            ResourceType::Memory(d) => d.get_id(),
            ResourceType::GPU(d) => d.get_id(),
            ResourceType::Drive(d) => d.get_id(),
            ResourceType::Network(e) => e.get_id(),
            ResourceType::Battery(bat) => bat.get_id(),
            ResourceType::Process(p) => p.get_id(),
        }
    }

    pub fn get_type_name(&self) -> &'static str {
        match self {
            ResourceType::CPU(rt) => rt.get_type_name(),
            ResourceType::Memory(rt) => rt.get_type_name(),
            ResourceType::GPU(rt) => rt.get_type_name(),
            ResourceType::Drive(rt) => rt.get_type_name(),
            ResourceType::Network(rt) => rt.get_type_name(),
            ResourceType::Battery(rt) => rt.get_type_name(),
            ResourceType::Process(rt) => rt.get_type_name(),
        }
    }

    pub fn get_name(&self) -> String {
        match self {
            ResourceType::CPU(rt) => rt.get_name(),
            ResourceType::Memory(rt) => rt.get_name(),
            ResourceType::GPU(rt) => rt.get_name(),
            ResourceType::Drive(rt) => rt.get_name(),
            ResourceType::Network(rt) => rt.get_name(),
            ResourceType::Battery(rt) => rt.get_name(),
            ResourceType::Process(rt) => rt.get_name(),
        }
    }

    pub fn handle_navi_event(&mut self, event: &NavigatorEvent) -> bool {
        match self {
            ResourceType::CPU(cpu) => cpu.handle_navi_event(event),
            ResourceType::Memory(mem) => mem.handle_navi_event(event),
            ResourceType::GPU(gpu) => gpu.handle_navi_event(event),
            ResourceType::Drive(drive) => drive.handle_navi_event(event),
            ResourceType::Network(network) => network.handle_navi_event(event),
            ResourceType::Battery(b) => b.handle_navi_event(event),
            ResourceType::Process(p) => p.handle_navi_event(event),
        }
    }

    pub fn block(&self, args: &mut BlockArg) -> AResult<GroupedLines<'static>> {
        match self {
            ResourceType::CPU(rt) => rt.block(args),
            ResourceType::Memory(rt) => rt.block(args),
            ResourceType::GPU(rt) => rt.block(args),
            ResourceType::Drive(rt) => rt.block(args),
            ResourceType::Network(rt) => rt.block(args),
            ResourceType::Battery(rt) => rt.block(args),
            ResourceType::Process(rt) => rt.block(args),
        }
    }

    pub fn hides_sidebar(&self) -> bool {
        matches!(self, ResourceType::Process(process) if process.hides_sidebar())
    }

    pub fn render_header(&self, frame: &mut Frame, rect: Rect) {
        if rect.height == 0 {
            return;
        }

        let header_rect = Rect { height: 1, ..rect };
        let type_name = self.get_type_name();
        let name = self.get_name();
        let header = if !name.is_empty() {
            Line::from(vec![
                Span::styled(type_name, Style::new().add_modifier(Modifier::BOLD)),
                Span::raw("::"),
                Span::raw(name),
            ])
        } else {
            Span::styled(type_name, Style::new().add_modifier(Modifier::BOLD)).into()
        };
        frame.render_widget(header, header_rect);
    }

    pub fn render_process_list(&mut self, frame: &mut Frame, args: &DetailArg) {
        if let ResourceType::Process(process) = self {
            process.render_process_list(frame, args);
        }
    }

    pub fn render_process_detail(&mut self, frame: &mut Frame, args: &DetailArg) {
        if let ResourceType::Process(process) = self {
            process.render_process_detail(frame, args);
        }
    }

    pub fn render_detail(&mut self, frame: &mut Frame, args: &mut DetailArg) {
        let rect = args.rect;
        self.render_header(frame, rect);

        if rect.height > 1 {
            let content_rect = Rect {
                y: rect.y.saturating_add(1),
                height: rect.height.saturating_sub(1),
                ..rect
            };

            let mut args = args.clone();
            args.rect = content_rect;

            const MAX_WIDTH: u16 = 90;

            match self {
                ResourceType::CPU(rt) => rt.render_detail(frame, &args, MAX_WIDTH),
                ResourceType::Memory(rt) => rt.render_detail(frame, &args, MAX_WIDTH),
                ResourceType::GPU(rt) => rt.render_detail(frame, &args, MAX_WIDTH),
                ResourceType::Drive(rt) => rt.render_detail(frame, &args, MAX_WIDTH),
                ResourceType::Network(rt) => rt.render_detail(frame, &args, MAX_WIDTH),
                ResourceType::Battery(rt) => rt.render_detail(frame, &args, MAX_WIDTH),
                ResourceType::Process(rt) => rt.render_detail(frame, &args, MAX_WIDTH),
            };
        }
    }

    pub fn cached_page_state<'b>(&'b mut self) -> StatefulLinesType<'static, 'b> {
        match self {
            ResourceType::CPU(rt) => rt.cached_page_state(),
            ResourceType::Memory(rt) => rt.cached_page_state(),
            ResourceType::GPU(rt) => rt.cached_page_state(),
            ResourceType::Drive(rt) => rt.cached_page_state(),
            ResourceType::Network(rt) => rt.cached_page_state(),
            ResourceType::Battery(rt) => rt.cached_page_state(),
            ResourceType::Process(rt) => rt.cached_page_state(),
        }
    }

    pub fn updata_data(&mut self, rsp: &SensorRsp) -> bool {
        let rsp_id = rsp.get_id();
        match rsp {
            SensorRsp::CPU(data) => {
                if let ResourceType::CPU(rt) = self {
                    if rsp_id == rt.get_id() {
                        rt.update_data(data);
                        return true;
                    }
                }
            }
            SensorRsp::Memory(data) => {
                if let ResourceType::Memory(rt) = self {
                    if rsp_id == rt.get_id() {
                        rt.update_data(data);
                        return true;
                    }
                }
            }
            SensorRsp::GPU(data) => {
                if let ResourceType::GPU(rt) = self {
                    match data {
                        Ok(data) => {
                            if rsp_id == rt.get_id() {
                                rt.update_data(data);
                                return true;
                            }
                        }
                        Err(err) => {
                            log::error!("unable to read GPU data: {}, {}", err, err.backtrace());
                        }
                    }
                }
            }
            SensorRsp::Drive(data) => {
                if let ResourceType::Drive(rt) = self {
                    if rsp_id == rt.get_id() {
                        rt.update_data(data);
                        return true;
                    }
                }
            }
            SensorRsp::Network(data) => {
                if let ResourceType::Network(rt) = self {
                    if rsp_id == rt.get_id() {
                        rt.update_data(data);

                        return true;
                    }
                }
            }
            SensorRsp::Battery(data) => {
                if let ResourceType::Battery(rt) = self {
                    if rsp_id == rt.get_id() {
                        rt.update_data(data);
                        return true;
                    }
                }
            }
            SensorRsp::Process(data) => {
                if let ResourceType::Process(rt) = self {
                    rt.update_data(data);
                    return true;
                }
            }
        }
        false
    }
}

fn map_all_unique<V, F>(vs: impl Iterator<Item = V>, f: F) -> Vec<String>
where
    F: Fn(V) -> String,
{
    vs.map(f).unique().collect()
}

pub struct HardwareWorker {
    pub tx: Sender<SensorReq>,
}

impl HardwareWorker {
    pub fn spawn(result_tx: &Sender<ResourceEvent>) -> Self {
        // Bound the queue so a slow consumer cannot grow unbounded memory.
        let (tx, rx) = flume::bounded(64);

        let result_tx = result_tx.clone();
        thread::Builder::new()
            .name("sensorworker".to_owned())
            .spawn(move || loop {
                if let Ok(req) = rx.recv() {
                    let rsp = match req {
                        SensorReq::CPU(req) => ResCPU::do_sensor(req),
                        SensorReq::Memory(req) => ResMEM::do_sensor(req),
                        SensorReq::GPU(req) => ResGPU::do_sensor(req),
                        SensorReq::Drive(req) => ResDrive::do_sensor(req),
                        SensorReq::Network(req) => ResNetwork::do_sensor(req),
                        SensorReq::Battery(req) => ResBattery::do_sensor(req),
                        SensorReq::Process(req) => ResProcess::do_sensor(req),
                    };

                    if let Ok(SensorResultType::SyncResult(rsp)) = rsp {
                        let _ = result_tx.send(ResourceEvent::SensorRsp(rsp));
                    }
                }
            })
            .unwrap();

        Self { tx }
    }
}
