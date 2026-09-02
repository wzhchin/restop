use std::{
    cell::Cell,
    cmp::Ordering,
    sync::{Arc, RwLock},
};

use chin_tools::AResult;
use crossterm::event::{KeyCode, KeyModifiers};
use flume::Sender;

use once_cell::sync::Lazy;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders},
};
use unicode_width::UnicodeWidthChar;

use crate::{
    app::ResourceEvent,
    component::{
        grouped_lines::GroupedLines,
        input::Input,
        ls_kv, render_border, s_label,
        stateful_lines::{StatefulColumn, StatefulLinesType},
    },
    resource::SensorRsp,
    sensor::{
        process::{
            load_process_details, read_process_starttime_ticks, request_sample,
            send_process_action, LoadAvg, ProcessAction, ProcessItem, ProcessSensor,
            ProcessSnapshot,
        },
        process_data::Containerization,
        units::{conver_storage_width4, convert_seconds, convert_speed, convert_storage},
    },
    tarits::{format_fraction_as_percent, format_percent_number, None2NaN, None2NanString},
    utils::{is_alt_char, is_char_and_mod, is_esc},
    view::{theme::SharedTheme, BlockArg, DetailArg, NavigatorEvent},
};

use super::{Resource, SensorResultType};

pub const PROCESS_ID: &str = "PROCESS";

static PROCESS_SORT_TYPE: Lazy<RwLock<Option<(ProcessCell, bool)>>> =
    Lazy::new(|| RwLock::new(None));

#[derive(Debug)]
struct ProcessDetail {
    item: ProcessItem,
    executable: String,
    workdir: String,
    environment: Vec<String>,
    environment_truncated: bool,
}

impl ProcessDetail {
    fn load(item: ProcessItem) -> Self {
        let details = load_process_details(item.pid);

        Self {
            item,
            executable: details.executable,
            workdir: details.workdir,
            environment: details.environment,
            environment_truncated: details.environment_truncated,
        }
    }
}

fn process_action_name(action: ProcessAction) -> &'static str {
    match action {
        ProcessAction::TERM => "SIGTERM",
        ProcessAction::INT => "SIGINT",
        ProcessAction::HUP => "SIGHUP",
        ProcessAction::STOP => "SIGSTOP",
        ProcessAction::KILL => "SIGKILL",
        ProcessAction::CONT => "SIGCONT",
    }
}

fn containerization_name(value: Containerization) -> &'static str {
    match value {
        Containerization::None => "None",
        Containerization::Flatpak => "Flatpak",
        Containerization::Snap => "Snap",
    }
}

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
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProcessPanelFocus {
    List,
    Detail,
}

#[derive(Debug)]
pub struct ResProcess {
    data: Option<Arc<Vec<ProcessItem>>>,
    sampled_data: Option<Arc<Vec<ProcessItem>>>,
    loadavg: Option<LoadAvg>,
    uptime: Cell<u64>,
    theme: SharedTheme,

    view_state: StatefulColumn<'static>,
    line_builder: LineBuilder,
    filter: Option<Input>,
    detail: Option<ProcessDetail>,
    process_focus: ProcessPanelFocus,
    detail_lines: Vec<Line<'static>>,
    detail_line_width: u16,
    detail_scroll: usize,
    pending_signal: Option<ProcessAction>,
    detail_status: Option<String>,
}

#[derive(Debug)]
pub enum ProcessRsp {
    Snapshot(ProcessSnapshot),
    Processes(Arc<Vec<ProcessItem>>),
    LoadAvg(LoadAvg),
    Uptime(u64),
}

impl ResProcess {
    pub fn spawn(theme: SharedTheme, result_tx: &Sender<ResourceEvent>) -> AResult<Self> {
        let result_tx = result_tx.clone();
        ProcessSensor::spawn(move |snapshot| {
            let rsp = SensorRsp::Process(ProcessRsp::Snapshot(snapshot)).into();
            let _ = result_tx.send(ResourceEvent::SensorRsp(rsp));
        })?;
        Ok(Self {
            data: None,
            sampled_data: None,
            theme,
            loadavg: Default::default(),
            uptime: Default::default(),
            view_state: StatefulColumn::new(),
            line_builder: LineBuilder::new(),
            filter: None,
            detail: None,
            process_focus: ProcessPanelFocus::List,
            detail_lines: vec![],
            detail_line_width: 0,
            detail_scroll: 0,
            pending_signal: None,
            detail_status: None,
        })
    }

    /// Derive the one list used by both the sidebar count and the process
    /// page from the latest sensor-owned snapshot.
    fn rebuild_visible_data(&mut self) {
        let Some(sampled_data) = self.sampled_data.as_ref() else {
            self.data = None;
            self.view_state.mark_dirty();
            return;
        };

        let filter = self.filter.as_ref().map(Input::get_input);
        let sort = get_process_sort();
        if filter.as_deref().is_none_or(str::is_empty) && sort.is_none() {
            self.data = Some(sampled_data.clone());
            self.view_state.mark_dirty();
            return;
        }

        let mut visible = sampled_data.as_ref().clone();
        if let Some(filter) = filter {
            if !filter.is_empty() {
                visible.retain(|item| item.commandline.contains(&filter));
            }
        }
        if let Some((cell, desc)) = sort {
            visible.sort_unstable_by(|first, second| cell.cmp(first, second, desc));
        }

        self.data = Some(Arc::new(visible));
        self.view_state.mark_dirty();
    }

    fn update_processes(&mut self, process_data: Arc<Vec<ProcessItem>>) {
        let selected_index = self
            .process_list_focused()
            .then(|| self.view_state.focused_index())
            .flatten();
        if let Some(detail) = self.detail.as_ref() {
            let pid = detail.item.pid;
            let starttime = detail.item.starttime;
            let starttime_ticks = detail.item.starttime_ticks;
            if !process_data.iter().any(|item| {
                item.pid == pid
                    && item.starttime == starttime
                    && item.starttime_ticks == starttime_ticks
            }) {
                self.pending_signal = None;
                self.detail_status = Some(format!("PID {pid} exited or was replaced"));
            }
        }

        self.sampled_data = Some(process_data);
        self.rebuild_visible_data();
        if let Some(selected_index) = selected_index {
            let selected = self
                .data
                .as_ref()
                .and_then(|items| items.get(selected_index))
                .cloned();
            self.rebind_detail_to(selected);
        }
    }

    fn selected_process(&self) -> Option<ProcessItem> {
        let index = self.view_state.focused_index()?;
        self.data.as_ref()?.get(index).cloned()
    }

    fn process_list_focused(&self) -> bool {
        self.process_focus == ProcessPanelFocus::List
    }

    fn process_detail_focused(&self) -> bool {
        self.detail.is_some() && self.process_focus == ProcessPanelFocus::Detail
    }

    pub(crate) fn hides_sidebar(&self) -> bool {
        self.detail.is_some()
    }

    fn set_process_focus(&mut self, focus: ProcessPanelFocus) {
        if self.process_focus != focus {
            self.process_focus = focus;
            self.view_state.mark_dirty();
            self.detail_line_width = 0;
        }
    }

    fn reset_detail_view(&mut self) {
        self.detail_lines.clear();
        self.detail_line_width = 0;
        self.detail_scroll = 0;
        self.pending_signal = None;
        self.detail_status = None;
    }

    fn set_detail(&mut self, item: ProcessItem) {
        self.detail = Some(ProcessDetail::load(item));
        self.reset_detail_view();
    }

    fn open_selected_detail(&mut self) -> bool {
        let Some(item) = self.selected_process() else {
            return false;
        };
        self.set_detail(item);
        self.set_process_focus(ProcessPanelFocus::Detail);
        true
    }

    fn close_detail(&mut self) {
        self.detail = None;
        self.set_process_focus(ProcessPanelFocus::List);
        self.reset_detail_view();
    }

    fn rebind_detail_to(&mut self, item: Option<ProcessItem>) {
        let Some(item) = item else {
            return;
        };
        let Some(detail) = self.detail.as_ref() else {
            return;
        };
        let already_showing = detail.item.pid == item.pid
            && detail.item.starttime == item.starttime
            && detail.item.starttime_ticks == item.starttime_ticks;
        if !already_showing {
            self.set_detail(item);
        }
    }

    fn move_process_selection(&mut self, next: bool) {
        if next {
            self.view_state.focus_next();
        } else {
            self.view_state.focus_prev();
        }
        self.rebind_detail_to(self.selected_process());
    }

    fn refresh_detail(&mut self) {
        let Some(previous) = self.detail.as_ref() else {
            return;
        };
        let pid = previous.item.pid;
        let starttime = previous.item.starttime;
        let starttime_ticks = previous.item.starttime_ticks;
        let Some(item) = self.data.as_ref().and_then(|items| {
            items
                .iter()
                .find(|item| {
                    item.pid == pid
                        && item.starttime == starttime
                        && item.starttime_ticks == starttime_ticks
                })
                .cloned()
        }) else {
            self.detail_status = Some(format!("PID {pid} exited or was replaced"));
            return;
        };

        self.detail = Some(ProcessDetail::load(item));
        self.detail_lines.clear();
        self.detail_line_width = 0;
        self.detail_status = Some(format!("Refreshed PID {pid}"));
    }

    fn request_signal(&mut self, action: ProcessAction) {
        let Some(detail) = self.detail.as_ref() else {
            return;
        };
        let warning = if action == ProcessAction::KILL {
            "cannot be handled or undone"
        } else {
            "may terminate or alter the process"
        };
        self.pending_signal = Some(action);
        self.detail_status = Some(format!(
            "Confirm {} -> PID {} ({}): {warning}. y=yes, n=no",
            process_action_name(action),
            detail.item.pid,
            detail.item.display_name
        ));
    }

    fn confirm_signal(&mut self) {
        let Some(action) = self.pending_signal.take() else {
            return;
        };
        let Some(detail) = self.detail.as_ref() else {
            return;
        };
        let pid = detail.item.pid;
        let starttime = detail.item.starttime;
        let starttime_ticks = detail.item.starttime_ticks;
        let still_same_process = self.data.as_ref().is_some_and(|items| {
            items.iter().any(|item| {
                item.pid == pid
                    && item.starttime == starttime
                    && item.starttime_ticks == starttime_ticks
            })
        });
        if !still_same_process {
            self.detail_status = Some(format!(
                "Aborted {}: PID {pid} exited or was replaced",
                process_action_name(action)
            ));
            return;
        }

        match read_process_starttime_ticks(pid) {
            Ok(Some(current)) if current == starttime_ticks => {}
            Ok(_) => {
                self.detail_status = Some(format!(
                    "Aborted {}: PID {pid} start time changed",
                    process_action_name(action)
                ));
                return;
            }
            Err(error) => {
                self.detail_status = Some(format!(
                    "Aborted {}: cannot verify PID {pid}: {error}",
                    process_action_name(action)
                ));
                return;
            }
        }

        self.detail_status = Some(match send_process_action(pid, action) {
            Ok(()) => format!("Sent {} to PID {pid}", process_action_name(action)),
            Err(error) => format!(
                "Failed {} -> PID {pid}: {error}",
                process_action_name(action)
            ),
        });
    }

    fn rebuild_detail_lines(&mut self, width: u16) {
        if self.detail_line_width == width && !self.detail_lines.is_empty() {
            return;
        }
        let Some(detail) = self.detail.as_ref() else {
            return;
        };
        let key_style = self.theme.key(true);
        let value_style = self.theme.value(true);
        let mut lines = vec![Line::raw(
            "↑/↓ scroll/select · r refresh · t/i/h/k/s/c signal · ← focus list/close · → focus detail",
        )];
        let mut push_kv = |key: &str, value: String| {
            lines.extend(ls_kv(Some(key), &value, width, key_style, value_style));
        };

        push_kv("PID", detail.item.pid.to_string());
        push_kv("User", detail.item.user.clone());
        push_kv("Name", detail.item.display_name.clone());
        push_kv("Command", detail.item.commandline.clone());
        push_kv("Executable", detail.executable.clone());
        push_kv("Workdir", detail.workdir.clone());
        push_kv(
            "Container",
            containerization_name(detail.item.containerization).to_owned(),
        );
        push_kv(
            "Cgroup",
            detail
                .item
                .cgroup
                .clone()
                .unwrap_or_else(|| "N/A".to_owned()),
        );
        push_kv(
            "Memory",
            convert_storage(detail.item.memory_usage as f64, false),
        );
        push_kv(
            "CPU",
            format_fraction_as_percent(detail.item.cpu_time_ratio as f64),
        );
        push_kv("User CPU", format!("{:.2} s", detail.item.user_cpu_time));
        push_kv(
            "System CPU",
            format!("{:.2} s", detail.item.system_cpu_time),
        );
        push_kv(
            "Started",
            format!("boot + {}", convert_seconds(detail.item.starttime as u64)),
        );
        push_kv(
            "Read",
            detail
                .item
                .read_speed
                .map(|value| convert_speed(value, false))
                .unwrap_or_else(|| "N/A".to_owned()),
        );
        push_kv(
            "Read Total",
            detail
                .item
                .read_total
                .map(|value| convert_storage(value as f64, false))
                .unwrap_or_else(|| "N/A".to_owned()),
        );
        push_kv(
            "Write",
            detail
                .item
                .write_speed
                .map(|value| convert_speed(value, false))
                .unwrap_or_else(|| "N/A".to_owned()),
        );
        push_kv(
            "Write Total",
            detail
                .item
                .write_total
                .map(|value| convert_storage(value as f64, false))
                .unwrap_or_else(|| "N/A".to_owned()),
        );
        push_kv("Process GPU", "N/A (collection disabled)".to_owned());

        lines.push(Line::raw(""));
        lines.push(Line::styled(
            format!(
                "Environment ({}{})",
                detail.environment.len(),
                if detail.environment_truncated {
                    ", truncated"
                } else {
                    ""
                }
            ),
            key_style,
        ));
        for entry in &detail.environment {
            lines.extend(ls_kv(None, entry, width, key_style, value_style));
        }

        self.detail_lines = lines;
        self.detail_line_width = width;
    }

    fn render_detail_drawer(&mut self, frame: &mut ratatui::Frame, rect: Rect, focused: bool) {
        if rect.width == 0 || rect.height == 0 {
            return;
        }
        let title = self
            .detail
            .as_ref()
            .map(|detail| format!("Process {}", detail.item.pid))
            .unwrap_or_else(|| "Process".to_owned());
        let border_type = if focused {
            BorderType::Double
        } else {
            BorderType::Plain
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(border_type)
            .border_style(self.theme.border(focused))
            .title(title);
        let inner = block.inner(rect);
        frame.render_widget(block, rect);
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        self.rebuild_detail_lines(inner.width);
        let status_height = u16::from(self.detail_status.is_some());
        let content_height = inner.height.saturating_sub(status_height);
        let max_scroll = self
            .detail_lines
            .len()
            .saturating_sub(content_height as usize);
        self.detail_scroll = self.detail_scroll.min(max_scroll);
        for (offset, line) in self
            .detail_lines
            .iter()
            .skip(self.detail_scroll)
            .take(content_height as usize)
            .enumerate()
        {
            frame.render_widget(
                line,
                Rect {
                    x: inner.x,
                    y: inner.y.saturating_add(offset as u16),
                    width: inner.width,
                    height: 1,
                },
            );
        }
        if let Some(status) = self.detail_status.as_ref() {
            frame.render_widget(
                Line::styled(status.as_str(), Style::new().fg(Color::Black)),
                Rect {
                    x: inner.x,
                    y: inner.bottom().saturating_sub(1),
                    width: inner.width,
                    height: 1,
                },
            );
        }
    }

    pub(crate) fn render_process_list(&mut self, frame: &mut ratatui::Frame, args: &DetailArg) {
        if args.rect.width == 0 || args.rect.height == 0 {
            return;
        }

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

        if content_rect.width > 0 && content_rect.height > 0 {
            let list_args = DetailArg {
                rect: content_rect,
                ..args.clone()
            };
            match self._build_page(&list_args) {
                Ok(_) => {}
                Err(err) => {
                    log::error!("unable to render_page: {}", err);
                    return;
                }
            }

            let lines = self.cached_page_state();
            if let StatefulLinesType::Lines(lines) = lines {
                lines.render(frame, content_rect);
            }
        }

        render_border(
            args.active && self.process_list_focused(),
            args.rect,
            frame.buffer_mut(),
        );
    }

    pub(crate) fn render_process_detail(&mut self, frame: &mut ratatui::Frame, args: &DetailArg) {
        self.render_detail_drawer(frame, args.rect, self.process_detail_focused());
    }
}

impl Resource for ResProcess {
    type Req = ();

    type Rsp = ProcessRsp;

    fn do_sensor(_: Self::Req) -> AResult<SensorResultType> {
        request_sample();
        Ok(SensorResultType::AsyncResult)
    }

    fn get_id(&self) -> &str {
        PROCESS_ID
    }

    fn get_req(&self) -> Self::Req {}

    fn block(&self, args: &mut BlockArg) -> AResult<GroupedLines<'static>> {
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

    fn _build_page(&mut self, args: &DetailArg) -> AResult<String> {
        self.view_state.update_view_height(args.rect.height);
        if self.view_state.is_dirty() {
            self.view_state.set_header(self.line_builder.to_header());
            if let Some(data) = self.data.as_ref() {
                let list_focused = self.process_list_focused();
                let line_builder = &self.line_builder;
                let fg = self.theme.fg();
                self.view_state.update_lines(data, |e, s| {
                    line_builder.to_line(e, s && list_focused).fg(fg)
                });
            }
        }

        Ok("Process".to_string())
    }

    fn update_data(&mut self, data: &Self::Rsp) {
        match data {
            ProcessRsp::Snapshot(snapshot) => {
                if let Some(loadavg) = snapshot.loadavg.as_ref() {
                    self.loadavg = Some(loadavg.clone());
                }
                if let Some(uptime) = snapshot.uptime {
                    self.uptime.set(uptime);
                }
                self.update_processes(snapshot.processes.clone());
            }
            ProcessRsp::Processes(process_data) => self.update_processes(process_data.clone()),
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
                if self.process_list_focused() {
                    if let Some(input) = self.filter.as_mut() {
                        let handled = input.handle_event(ke);
                        if handled {
                            self.rebuild_visible_data();
                            return true;
                        }

                        if is_esc(ke) {
                            self.filter.take();
                            self.rebuild_visible_data();
                            return true;
                        }
                    }

                    if is_char_and_mod(ke, '/', KeyModifiers::NONE)
                        || is_char_and_mod(ke, 's', KeyModifiers::CONTROL)
                    {
                        self.filter.replace(Input::new());
                        self.rebuild_visible_data();
                        return true;
                    }
                }

                if self.detail.is_some() {
                    if self.pending_signal.is_some() {
                        match ke.code {
                            KeyCode::Char('y') => self.confirm_signal(),
                            KeyCode::Char('n') | KeyCode::Esc => {
                                self.pending_signal = None;
                                self.detail_status = Some("Signal cancelled".to_owned());
                            }
                            _ => {}
                        }
                        return true;
                    }

                    if self.process_detail_focused() {
                        match ke.code {
                            KeyCode::Left => {
                                self.set_process_focus(ProcessPanelFocus::List);
                            }
                            KeyCode::Esc => self.close_detail(),
                            KeyCode::Up => {
                                self.detail_scroll = self.detail_scroll.saturating_sub(1)
                            }
                            KeyCode::Down => {
                                self.detail_scroll = self.detail_scroll.saturating_add(1)
                            }
                            KeyCode::PageUp => {
                                self.detail_scroll = self.detail_scroll.saturating_sub(10)
                            }
                            KeyCode::PageDown => {
                                self.detail_scroll = self.detail_scroll.saturating_add(10)
                            }
                            KeyCode::Char('r') => self.refresh_detail(),
                            KeyCode::Char('t') => self.request_signal(ProcessAction::TERM),
                            KeyCode::Char('i') => self.request_signal(ProcessAction::INT),
                            KeyCode::Char('h') => self.request_signal(ProcessAction::HUP),
                            KeyCode::Char('k') => self.request_signal(ProcessAction::KILL),
                            KeyCode::Char('s') => self.request_signal(ProcessAction::STOP),
                            KeyCode::Char('c') => self.request_signal(ProcessAction::CONT),
                            _ => {}
                        }
                        return true;
                    }

                    match ke.code {
                        KeyCode::Left => {
                            self.close_detail();
                            return false;
                        }
                        KeyCode::Right => {
                            self.set_process_focus(ProcessPanelFocus::Detail);
                        }
                        KeyCode::Up => self.move_process_selection(false),
                        KeyCode::Down => self.move_process_selection(true),
                        KeyCode::Esc => self.close_detail(),
                        _ => {}
                    }
                    return true;
                }

                if ke.code == KeyCode::Right {
                    return self.open_selected_detail();
                }

                if is_alt_char(ke, 'p') {
                    try_change_sort(ProcessCell::PID);
                    self.rebuild_visible_data();
                    return true;
                }

                if is_alt_char(ke, 'c') {
                    try_change_sort(ProcessCell::CPU);
                    self.rebuild_visible_data();
                    return true;
                }

                if is_alt_char(ke, 'm') {
                    try_change_sort(ProcessCell::MEM);
                    self.rebuild_visible_data();
                    return true;
                }

                if is_alt_char(ke, 'n') {
                    try_change_sort(ProcessCell::CMD);
                    self.rebuild_visible_data();
                    return true;
                }
            }
        }
        false
    }

    fn get_name(&self) -> String {
        "".to_string()
    }

    fn render_detail(&mut self, frame: &mut ratatui::Frame, args: &DetailArg, _max_width: u16) {
        self.render_process_list(frame, args);
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, sync::Arc};

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{backend::TestBackend, layout::Rect, Terminal};

    use crate::{
        component::stateful_lines::StatefulColumn,
        resource::Resource,
        sensor::{
            process::{LoadAvg, ProcessItem, ProcessSnapshot},
            Containerization,
        },
        view::{theme::Theme, DetailArg, NavigatorEvent},
    };

    use super::{LineBuilder, ProcessPanelFocus, ProcessRsp, ResProcess};

    fn process_item(pid: i32) -> ProcessItem {
        ProcessItem {
            pid,
            user: "user".to_owned(),
            display_name: format!("process-{pid}"),
            memory_usage: 1024,
            cpu_time_ratio: 0.1,
            user_cpu_time: 1.0,
            system_cpu_time: 0.5,
            commandline: format!("process-{pid}"),
            containerization: Containerization::None,
            starttime: pid as f64,
            starttime_ticks: pid as u64,
            cgroup: None,
            read_speed: None,
            read_total: None,
            write_speed: None,
            write_total: None,
            gpu_usage: 0.0,
            enc_usage: 0.0,
            dec_usage: 0.0,
            gpu_mem_usage: 0,
        }
    }

    fn process_with_items(items: Vec<ProcessItem>) -> ResProcess {
        let sampled_data = Arc::new(items.clone());
        let mut process = ResProcess {
            data: Some(Arc::new(items)),
            sampled_data: Some(sampled_data),
            loadavg: None,
            uptime: Cell::new(0),
            theme: Arc::new(Theme::default()),
            view_state: StatefulColumn::new(),
            line_builder: LineBuilder::new(),
            filter: None,
            detail: None,
            process_focus: ProcessPanelFocus::List,
            detail_lines: vec![],
            detail_line_width: 0,
            detail_scroll: 0,
            pending_signal: None,
            detail_status: None,
        };
        process.view_state.update_view_height(20);
        process.view_state.mark_dirty();
        process
            ._build_page(&DetailArg {
                rect: Rect::new(0, 0, 80, 20),
                active: true,
            })
            .unwrap();
        process
    }

    fn key(code: KeyCode) -> NavigatorEvent {
        NavigatorEvent::KeyEvent(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn process_detail_navigation_has_separate_focus_and_visibility() {
        let mut process = process_with_items(vec![process_item(1), process_item(2)]);

        assert!(!process.hides_sidebar());
        assert!(process.handle_navi_event(&key(KeyCode::Right)));
        assert_eq!(process.process_focus, ProcessPanelFocus::Detail);
        assert!(process.detail.is_some());
        assert!(process.hides_sidebar());

        assert!(process.handle_navi_event(&key(KeyCode::Left)));
        assert_eq!(process.process_focus, ProcessPanelFocus::List);
        assert!(process.detail.is_some());

        assert!(process.handle_navi_event(&key(KeyCode::Down)));
        assert_eq!(process.view_state.focused_index(), Some(1));
        assert_eq!(
            process.detail.as_ref().map(|detail| detail.item.pid),
            Some(2)
        );

        assert!(!process.handle_navi_event(&key(KeyCode::Left)));
        assert_eq!(process.process_focus, ProcessPanelFocus::List);
        assert!(process.detail.is_none());
        assert!(!process.hides_sidebar());
    }

    #[test]
    fn process_list_refresh_stays_dirty_and_rebinds_visible_detail() {
        let mut process = process_with_items(vec![process_item(1), process_item(2)]);
        assert!(process.handle_navi_event(&key(KeyCode::Right)));
        assert!(process.handle_navi_event(&key(KeyCode::Left)));

        process.view_state.mark_dirty();
        process
            ._build_page(&DetailArg {
                rect: Rect::new(0, 0, 80, 20),
                active: true,
            })
            .unwrap();
        assert!(!process.view_state.is_dirty());

        process.update_data(&ProcessRsp::Processes(Arc::new(vec![
            process_item(3),
            process_item(4),
        ])));

        assert!(process.view_state.is_dirty());
        assert_eq!(
            process.detail.as_ref().map(|detail| detail.item.pid),
            Some(3)
        );
    }

    #[test]
    fn process_list_refresh_does_not_open_closed_detail() {
        let mut process = process_with_items(vec![process_item(1)]);

        process.update_data(&ProcessRsp::Processes(Arc::new(vec![process_item(2)])));

        assert!(process.detail.is_none());
    }

    #[test]
    fn focused_process_list_can_filter_with_detail_open() {
        let mut process = process_with_items(vec![process_item(1), process_item(2)]);

        assert!(process.handle_navi_event(&key(KeyCode::Right)));
        assert!(process.handle_navi_event(&key(KeyCode::Left)));
        assert_eq!(process.process_focus, ProcessPanelFocus::List);

        assert!(
            process.handle_navi_event(&NavigatorEvent::KeyEvent(KeyEvent::new(
                KeyCode::Char('s'),
                KeyModifiers::CONTROL,
            )))
        );
        assert!(process.filter.is_some());

        assert!(process.handle_navi_event(&key(KeyCode::Char('2'))));
        assert_eq!(process.data.as_ref().unwrap().len(), 1);
        assert_eq!(process.data.as_ref().unwrap()[0].pid, 2);
        assert_eq!(process.sampled_data.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn slash_activates_process_filter_when_list_is_focused() {
        let mut process = process_with_items(vec![process_item(1)]);

        assert!(process.handle_navi_event(&key(KeyCode::Char('/'))));
        assert!(process.filter.is_some());
    }

    #[test]
    fn focused_process_detail_keeps_title_visible() {
        let mut process = process_with_items(vec![process_item(7)]);
        process.set_detail(process_item(7));
        process.set_process_focus(ProcessPanelFocus::Detail);

        let mut terminal = Terminal::new(TestBackend::new(40, 8)).unwrap();
        terminal
            .draw(|frame| {
                process.render_process_detail(
                    frame,
                    &DetailArg {
                        rect: Rect::new(0, 0, 40, 8),
                        active: true,
                    },
                );
            })
            .unwrap();

        let title_row: String = (0..40)
            .map(|x| terminal.backend().buffer()[(x, 0)].symbol().to_owned())
            .collect();
        assert!(title_row.contains("Process 7"), "{title_row:?}");
    }

    #[test]
    fn one_snapshot_updates_sidebar_and_process_list_data() {
        let mut process = process_with_items(vec![]);
        let item = process_item(7);

        process.update_data(&ProcessRsp::Snapshot(ProcessSnapshot {
            processes: Arc::new(vec![item]),
            loadavg: Some(LoadAvg {
                last1: 1.0,
                last5: 2.0,
                last15: 3.0,
                processes: "1/7".to_owned(),
            }),
            uptime: Some(42),
        }));

        assert_eq!(process.data.as_ref().unwrap().len(), 1);
        assert_eq!(process.sampled_data.as_ref().unwrap().len(), 1);
        assert_eq!(process.data.as_ref().unwrap()[0].pid, 7);
        assert_eq!(process.sampled_data.as_ref().unwrap()[0].pid, 7);
        assert!(Arc::ptr_eq(
            process.data.as_ref().unwrap(),
            process.sampled_data.as_ref().unwrap()
        ));
        assert_eq!(process.uptime.get(), 42);
        assert_eq!(process.loadavg.as_ref().unwrap().processes, "1/7");
    }

    #[test]
    fn filter_is_derived_from_the_latest_sensor_snapshot() {
        let mut process = process_with_items(vec![process_item(1), process_item(2)]);

        assert!(
            process.handle_navi_event(&NavigatorEvent::KeyEvent(KeyEvent::new(
                KeyCode::Char('s'),
                KeyModifiers::CONTROL,
            )))
        );
        assert!(process.handle_navi_event(&key(KeyCode::Char('2'))));

        assert_eq!(process.data.as_ref().unwrap().len(), 1);
        assert_eq!(process.data.as_ref().unwrap()[0].pid, 2);
        assert_eq!(process.sampled_data.as_ref().unwrap().len(), 2);
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
                self.keep_width(format_percent_number(data.cpu_time_ratio as f64 * 100.).as_str())
            }
            ProcessCell::READ => match data.read_speed.as_ref() {
                Some(o) => self.keep_width(conver_storage_width4(*o).as_str()),
                None => {
                    return s_label(
                        &self.keep_width(conver_storage_width4(0.).as_str()),
                        Style::new().fg(Color::Black),
                    )
                }
            },
            ProcessCell::WRITE => match data.write_speed.as_ref() {
                Some(o) => self.keep_width(conver_storage_width4(*o).as_str()),
                None => {
                    return s_label(
                        &self.keep_width(conver_storage_width4(0.).as_str()),
                        Style::new().fg(Color::Black),
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
