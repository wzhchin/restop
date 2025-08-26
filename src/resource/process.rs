use std::{
    cell::Cell,
    cmp::Ordering,
    sync::{Arc, RwLock},
    thread,
};

use chin_tools::AResult;
use crossterm::event::KeyModifiers;
use flume::{Receiver, Sender};

use itertools::Itertools;
use once_cell::sync::Lazy;
use process_data::ProcessData;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
};
use unicode_width::UnicodeWidthChar;

use crate::{
    app::ResourceEvent,
    component::{
        grouped_lines::GroupedLines,
        input::Input,
        render_border, s_label,
        stateful_lines::{StatefulColumn, StatefulLinesType},
    },
    resource::SensorRsp,
    sensor::{
        apps::AppsContext,
        process::{read_proc_loadavg, read_proc_uptime, LoadAvg, ProcessItem},
        units::{conver_storage_width4, convert_seconds},
    },
    tarits::{None2NaN, None2NanString},
    utils::{is_alt_char, is_char_and_mod, is_esc},
    view::{theme::SharedTheme, NavigatorEvent, OverviewArg, PageArg},
};

use super::{Resource, SensorResultType};

pub const PROCESS_ID: &str = "PROCESS";

static PROCESS_WORKER_CHANNEL: Lazy<(Sender<ProcessMsg>, Receiver<ProcessMsg>)> =
    Lazy::new(flume::unbounded);
static PROCESS_SORT_TYPE: Lazy<RwLock<Option<(ProcessCell, bool)>>> =
    Lazy::new(|| RwLock::new(None));

fn get_process_sort() -> Option<(ProcessCell, bool)> {
    *PROCESS_SORT_TYPE.read().unwrap()
}

fn try_change_sort(c: ProcessCell) {
    let sort = get_process_sort();

    if let Ok(mut write) = PROCESS_SORT_TYPE.write() {
        if let Some((cell, desc)) = sort {
            write.replace((c, if cell == c { !desc } else { true }));
        } else {
            write.replace((c, true));
        }
        let _ = PROCESS_WORKER_CHANNEL.0.send(ProcessMsg::ReadOnly);
    }
}

#[derive(Debug)]
pub struct ResProcess {
    data: Option<Arc<Vec<ProcessItem>>>,
    loadavg: Option<LoadAvg>,
    uptime: Cell<u64>,
    theme: SharedTheme,

    view_state: StatefulColumn<'static>,
    line_builder: LineBuilder,
    filter: Option<Input>,
}

#[derive(Debug)]
pub enum ProcessRsp {
    Processes(Arc<Vec<ProcessItem>>),
    LoadAvg(LoadAvg),
    Uptime(u64),
}

impl ResProcess {
    pub fn spawn(theme: SharedTheme, result_tx: &Sender<ResourceEvent>) -> AResult<Self> {
        ProcessWorker::spawn(result_tx)?;
        Ok(Self {
            data: None,
            theme,
            loadavg: Default::default(),
            uptime: Default::default(),
            view_state: StatefulColumn::new(),
            line_builder: LineBuilder::new(),
            filter: None,
        })
    }
}

impl Resource for ResProcess {
    type Req = ();

    type Rsp = ProcessRsp;

    fn do_sensor(_: Self::Req) -> AResult<SensorResultType> {
        PROCESS_WORKER_CHANNEL.0.send(ProcessMsg::Detect)?;
        Ok(SensorResultType::AsyncResult)
    }

    fn get_id(&self) -> &str {
        PROCESS_ID
    }

    fn get_req(&self) -> Self::Req {}

    fn overview_content(&self, args: &mut OverviewArg) -> AResult<GroupedLines<'static>> {
        let width = args.width;
        let block = GroupedLines::builder(width, &self.theme)
            .kv("Uptime", convert_seconds(self.uptime.get()))
            .kv("Load", self.loadavg.or_nan_owned())
            .kv(
                "Processes",
                self.data.or_nan(|e| {
                    format!(
                        "{} {}",
                        e.len(),
                        self.loadavg
                            .as_ref()
                            .map_or("".to_string(), |e| e.processes.to_string())
                    )
                }),
            )
            .active(args.focused)
            .build("Process")?;

        Ok(block)
    }

    fn _build_page(&mut self, args: &PageArg) -> AResult<String> {
        self.view_state.set_header(self.line_builder.to_header());
        self.view_state.update_view_height(args.rect.height);
        if let Some(data) = self.data.as_ref() {
            self.view_state.update_lines(data, |e, s| {
                self.line_builder.to_line(e, s).fg(self.theme.fg())
            });
        }

        Ok("Process".to_string())
    }

    fn update_data(&mut self, data: &Self::Rsp) {
        match data {
            ProcessRsp::Processes(process_data) => {
                self.data.replace(process_data.clone());
            }
            ProcessRsp::LoadAvg(load) => {
                self.loadavg.replace(load.clone());
            }
            ProcessRsp::Uptime(uptime) => {
                self.uptime.set(*uptime);
            }
        }
    }

    fn cached_page_state<'b>(&'b mut self) -> StatefulLinesType<'static, 'b> {
        StatefulLinesType::Lines(&mut self.view_state)
    }

    fn get_type_name(&self) -> &'static str {
        PROCESS_ID
    }

    fn handle_navi_event(&mut self, event: &NavigatorEvent) -> bool {
        match event {
            NavigatorEvent::KeyEvent(ke) => {
                if let Some(input) = self.filter.as_mut() {
                    let handled = input.handle_event(ke);
                    if handled {
                        let _ = PROCESS_WORKER_CHANNEL
                            .0
                            .send(ProcessMsg::Filter(input.get_input()));
                        return handled;
                    }
                };

                if is_esc(ke) {
                    self.filter.take();
                    let _ = PROCESS_WORKER_CHANNEL
                        .0
                        .send(ProcessMsg::Filter("".to_string()));
                    return true;
                }

                if is_char_and_mod(ke, 's', KeyModifiers::CONTROL) {
                    self.filter.replace(Input::new());
                    return true;
                }

                if is_alt_char(ke, 'p') {
                    try_change_sort(ProcessCell::PID);
                    return true;
                }

                if is_alt_char(ke, 'c') {
                    try_change_sort(ProcessCell::CPU);
                    return true;
                }

                if is_alt_char(ke, 'm') {
                    try_change_sort(ProcessCell::MEM);
                    return true;
                }

                if is_alt_char(ke, 'n') {
                    try_change_sort(ProcessCell::CMD);
                    return true;
                }
            }
        }
        false
    }

    fn get_name(&self) -> String {
        "".to_string()
    }

    fn render_page(&mut self, frame: &mut ratatui::Frame, args: &PageArg, _max_width: u16) {
        let inner = Rect {
            x: args.rect.x.saturating_add(1),
            y: args.rect.y.saturating_add(1),
            width: args.rect.width.saturating_sub(2),
            height: args.rect.height.saturating_sub(2),
        };

        if let Some(filter) = self.filter.as_mut() {
            let rect = Rect {
                y: inner.y,
                height: 1,
                ..inner
            };
            filter.draw(frame, &rect);
        }

        let content_rect = if self.filter.is_some() {
            Rect {
                y: inner.y.saturating_add(1),
                height: inner.height.saturating_sub(1),
                ..inner
            }
        } else {
            inner
        };
        if content_rect.height > 0 {
            match self._build_page(args) {
                Ok(_) => {}
                Err(err) => {
                    log::error!("unable to render_page: {}", err);
                    return;
                }
            }

            let lines = self.cached_page_state();

            if let StatefulLinesType::Lines(lines) = lines {
                lines.render(frame, content_rect)
            }
        }

        let buffer = frame.buffer_mut();
        render_border(args.active, args.rect, buffer);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(dead_code)]
#[allow(clippy::upper_case_acronyms)]
enum ProcessCell {
    PID,
    PRG,
    USER,
    CMD,
    MEM,
    CPU,
    READ,
    WRITE,
    TIME,
}

impl ProcessCell {
    fn width(&self) -> u16 {
        match self {
            ProcessCell::PID => 9,
            ProcessCell::PRG => 11,
            ProcessCell::USER => 6,
            ProcessCell::CMD => 200,
            ProcessCell::MEM => 7,
            ProcessCell::CPU => 6,
            ProcessCell::READ => 7,
            ProcessCell::WRITE => 7,
            ProcessCell::TIME => 20,
        }
    }

    fn keep_width<S>(&self, noodle: S) -> String
    where
        S: Into<String>,
    {
        let total_width = self.width();
        let content_width = total_width.saturating_sub(2);
        let mut noodle: String = noodle.into();

        let mut width: usize = 0;
        let mut size = None;

        for c in noodle.chars() {
            if let Some(wid) = c.width() {
                let nw = width.saturating_add(wid);
                if nw > content_width.into() {
                    size.replace(width);
                    break;
                }
                width = nw;
            }
        }

        if let Some(size) = size {
            noodle.truncate(size);
        }

        let padding = total_width.saturating_sub(width as u16);
        if padding > 0 {
            for _ in 0..padding {
                noodle.push(' ');
            }
        }

        noodle
    }

    fn to_value(self, data: &ProcessItem) -> Span<'static> {
        let s = match self {
            ProcessCell::PID => self.keep_width(data.pid.to_string().as_str()),
            ProcessCell::USER => self.keep_width(data.user.as_str()),
            ProcessCell::CMD => self.keep_width(data.commandline.as_str()),
            ProcessCell::MEM => {
                self.keep_width(conver_storage_width4(data.memory_usage as f64).as_str())
            }
            ProcessCell::CPU => {
                self.keep_width(format!("{:.1}", data.cpu_time_ratio * 100.).as_str())
            }
            ProcessCell::READ => match data.read_speed.as_ref() {
                Some(o) => self.keep_width(conver_storage_width4(*o).as_str()),
                None => {
                    return s_label(
                        &self.keep_width(conver_storage_width4(0.).as_str()),
                        Style::new().fg(Color::Blue),
                    )
                }
            },
            ProcessCell::WRITE => match data.write_speed.as_ref() {
                Some(o) => self.keep_width(conver_storage_width4(*o).as_str()),
                None => {
                    return s_label(
                        &self.keep_width(conver_storage_width4(0.).as_str()),
                        Style::new().fg(Color::Blue),
                    )
                }
            },
            ProcessCell::TIME => self.keep_width(data.starttime.to_string().as_str()),
            ProcessCell::PRG => self.keep_width(&data.display_name),
        };

        s.into()
    }

    fn to_label(self, suffix: char) -> Span<'static> {
        let mut s = String::new();
        let label = match self {
            ProcessCell::PID => "PID",
            ProcessCell::PRG => "NAME",
            ProcessCell::USER => "USER",
            ProcessCell::CMD => "CMD",
            ProcessCell::MEM => "MEM",
            ProcessCell::CPU => "CPU",
            ProcessCell::READ => "READ",
            ProcessCell::WRITE => "WRIT",
            ProcessCell::TIME => "TIME",
        };
        s.push_str(label);
        s.push(suffix);

        s = self.keep_width(s);
        s.into()
    }

    fn cmp(&self, f: &ProcessItem, b: &ProcessItem, desc: bool) -> Ordering {
        let r = match self {
            ProcessCell::PID => f.pid.cmp(&b.pid),
            ProcessCell::PRG => f.display_name.cmp(&b.display_name),
            ProcessCell::USER => f.user.cmp(&b.user),
            ProcessCell::CMD => f.commandline.cmp(&b.commandline),
            ProcessCell::MEM => f.memory_usage.cmp(&b.memory_usage),
            ProcessCell::CPU => f.cpu_time_ratio.total_cmp(&b.cpu_time_ratio),
            ProcessCell::READ => f
                .read_speed
                .partial_cmp(&b.read_speed)
                .unwrap_or(Ordering::Equal),
            ProcessCell::WRITE => f
                .write_speed
                .partial_cmp(&b.write_speed)
                .unwrap_or(Ordering::Equal),
            ProcessCell::TIME => Ordering::Equal,
        };

        if desc {
            r.reverse()
        } else {
            r
        }
    }
}

#[derive(Debug)]
struct LineBuilder {
    labels: Vec<ProcessCell>,
}

impl LineBuilder {
    pub fn new() -> Self {
        let header = vec![
            ProcessCell::PID,
            ProcessCell::USER,
            ProcessCell::CPU,
            ProcessCell::MEM,
            ProcessCell::READ,
            ProcessCell::WRITE,
            ProcessCell::CMD,
        ];

        Self { labels: header }
    }

    fn to_line(&self, item: &ProcessItem, active: bool) -> Line<'static> {
        let spans: Vec<Span<'static>> = self.labels.iter().map(|pc| pc.to_value(item)).collect();
        let line: Line<'static> = spans.into();
        if active {
            line.add_modifier(Modifier::REVERSED)
        } else {
            line
        }
    }

    fn to_header(&self) -> Line<'static> {
        let mut spans: Vec<Span<'static>> = vec![];
        let cmp = { *PROCESS_SORT_TYPE.read().unwrap() };

        for ele in self.labels.iter() {
            let suffix = if let Some((cell, desc)) = cmp {
                if cell == *ele {
                    if desc {
                        '↓'
                    } else {
                        '↑'
                    }
                } else {
                    ' '
                }
            } else {
                ' '
            };

            spans.push(ele.to_label(suffix))
        }
        spans.into()
    }
}

pub enum ProcessMsg {
    Detect,
    Filter(String),
    ReadOnly,
}

pub struct Process {}

pub struct ProcessWorker {
    app_context: AppsContext,
    filter: Option<String>,
}

impl ProcessWorker {
    pub fn spawn(result_tx: &Sender<ResourceEvent>) -> AResult<()> {
        let mut worker = ProcessWorker {
            app_context: AppsContext::new(),
            filter: None,
        };

        let req_rx = PROCESS_WORKER_CHANNEL.1.clone();
        let result_tx = result_tx.clone();

        thread::Builder::new()
            .name("processworker".to_owned())
            .spawn(move || loop {
                if let Ok(msg) = req_rx.recv() {
                    match msg {
                        ProcessMsg::Detect => {
                            if let Ok(uptime) = read_proc_uptime() {
                                let _ = result_tx.send(ResourceEvent::SensorRsp(
                                    SensorRsp::Process(ProcessRsp::Uptime(uptime)).into(),
                                ));
                            }
                            if let Ok(load) = read_proc_loadavg() {
                                let _ = result_tx.send(ResourceEvent::SensorRsp(
                                    SensorRsp::Process(ProcessRsp::LoadAvg(load)).into(),
                                ));
                            }
                            worker.updata_data();
                            let _ = result_tx.send(ResourceEvent::SensorRsp(
                                SensorRsp::Process(ProcessRsp::Processes(Arc::new(
                                    worker.get_process_items(),
                                )))
                                .into(),
                            ));
                        }
                        ProcessMsg::Filter(fileter) => {
                            if !fileter.is_empty() {
                                worker.filter.replace(fileter);
                            } else {
                                worker.filter.take();
                            }
                            let _ = result_tx.send(ResourceEvent::SensorRsp(
                                SensorRsp::Process(ProcessRsp::Processes(Arc::new(
                                    worker.get_process_items(),
                                )))
                                .into(),
                            ));
                        }
                        ProcessMsg::ReadOnly => {
                            let _ = result_tx.send(ResourceEvent::SensorRsp(
                                SensorRsp::Process(ProcessRsp::Processes(Arc::new(
                                    worker.get_process_items(),
                                )))
                                .into(),
                            ));
                        }
                    }
                }
            })?;

        Ok(())
    }

    pub fn updata_data(&mut self) {
        match ProcessData::all_process_data() {
            Ok(data) => {
                self.app_context.refresh(data);
            }
            Err(err) => {
                log::error!("unable to update process data: {}", err);
            }
        }
    }

    pub fn get_process_items(&self) -> Vec<ProcessItem> {
        let s = self
            .app_context
            .process_items()
            .into_iter()
            .map(|(_, v)| v)
            .filter(|e| {
                if let Some(f) = self.filter.as_ref() {
                    e.commandline.contains(f)
                } else {
                    true
                }
            });
        if let Some((cell, desc)) = get_process_sort() {
            s.sorted_by(|e1, e2| cell.cmp(e1, e2, desc)).collect()
        } else {
            s.collect()
        }
    }
}
