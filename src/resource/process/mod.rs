mod detail;
mod list;

use std::{cell::Cell, sync::Arc};

use chin_tools::AResult;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use detail::{process_action_name, ProcessDetail};
use flume::Sender;
use list::{try_change_sort, LineBuilder, ProcessCell};
use ratatui::{
    layout::Rect,
    style::Stylize,
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear},
};
use unicode_width::UnicodeWidthStr;

use crate::{
    app::ResourceEvent,
    component::{
        input::Input,
        overflow_block::OverflowBlock,
        stateful_lines::{StatefulColumn, StatefulLinesType},
    },
    resource::SensorRsp,
    sensor::{
        process::{
            read_process_starttime_ticks, request_sample, send_process_action, LoadAvg,
            ProcessAction, ProcessItem, ProcessSensor, ProcessSnapshot,
        },
        units::convert_seconds,
    },
    tarits::{None2NaN, None2NanString},
    utils::{is_alt_char, is_char_and_mod, is_esc},
    view::{theme::SharedTheme, BlockArg, DetailArg, NavigatorEvent},
};

use super::{Resource, SensorResultType};

pub const PROCESS_ID: &str = "PROCESS";

pub(super) fn is_help_key(key: &KeyEvent) -> bool {
    key.code == KeyCode::Char('?')
        && (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
}

pub(super) fn render_keys_popup(
    frame: &mut ratatui::Frame,
    area: Rect,
    theme: &SharedTheme,
    focused: bool,
    hints: &[(&str, &str)],
) {
    if area.width == 0 || area.height == 0 || hints.is_empty() {
        return;
    }
    let key_style = theme.key(focused);
    let value_style = theme.value(focused);
    let content_width = hints
        .iter()
        .map(|(key, action)| UnicodeWidthStr::width(*key) + UnicodeWidthStr::width(*action) + 4)
        .max()
        .unwrap_or(0) as u16;
    let popup_width = content_width
        .saturating_add(2)
        .clamp(24.min(area.width), area.width);
    let popup_height = (hints.len() as u16).saturating_add(2).min(area.height);
    let popup = Rect {
        x: area.x + area.width.saturating_sub(popup_width) / 2,
        y: area.y + area.height.saturating_sub(popup_height) / 2,
        width: popup_width,
        height: popup_height,
    };
    if popup.width < 3 || popup.height < 3 {
        return;
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(theme.border(true))
        .title(" Keys ");
    let inner = block.inner(popup);
    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);

    let key_column = hints
        .iter()
        .map(|(key, _)| UnicodeWidthStr::width(*key))
        .max()
        .unwrap_or(0);
    for (offset, (key, action)) in hints.iter().enumerate() {
        if offset >= inner.height as usize {
            break;
        }
        let pad = " ".repeat(key_column.saturating_sub(UnicodeWidthStr::width(*key)));
        frame.render_widget(
            Line::from(vec![
                Span::styled(format!("{key}{pad}"), key_style),
                Span::styled(format!("  {action}"), value_style),
            ]),
            Rect {
                x: inner.x,
                y: inner.y.saturating_add(offset as u16),
                width: inner.width,
                height: 1,
            },
        );
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
    detail_keys_hint: bool,
    list_keys_hint: bool,
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
            detail_keys_hint: false,
            list_keys_hint: false,
        })
    }

    /// Derive the one list used by both the sidebar count and the process
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
        self.detail_keys_hint = false;
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

    fn block(&self, args: &mut BlockArg) -> AResult<OverflowBlock<'static>> {
        let width = args.width;
        let block = OverflowBlock::builder(width, &self.theme)
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
                if self.detail.is_some() && self.detail_keys_hint {
                    if is_help_key(ke) || is_esc(ke) {
                        self.detail_keys_hint = false;
                    }
                    return true;
                }

                if self.list_keys_hint {
                    if is_help_key(ke) || is_esc(ke) {
                        self.list_keys_hint = false;
                    }
                    return true;
                }

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

                    if is_help_key(ke) {
                        self.list_keys_hint = true;
                        return true;
                    }
                }

                if self.detail.is_some() {
                    if self.process_detail_focused() && is_help_key(ke) {
                        self.detail_keys_hint = true;
                        return true;
                    }

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

    use super::{list::LineBuilder, ProcessPanelFocus, ProcessRsp, ResProcess};

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
            detail_keys_hint: false,
            list_keys_hint: false,
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

    fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
        let area = terminal.backend().buffer().area;
        let mut text = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                text.push_str(terminal.backend().buffer()[(x, y)].symbol());
            }
            text.push('\n');
        }
        text
    }

    #[test]
    fn process_detail_shows_keys_only_after_question_mark() {
        let mut process = process_with_items(vec![process_item(7)]);
        process.set_detail(process_item(7));
        process.set_process_focus(ProcessPanelFocus::Detail);

        let draw = |process: &mut ResProcess| {
            let mut terminal = Terminal::new(TestBackend::new(60, 30)).unwrap();
            terminal
                .draw(|frame| {
                    process.render_process_detail(
                        frame,
                        &DetailArg {
                            rect: Rect::new(0, 0, 60, 30),
                            active: true,
                        },
                    );
                })
                .unwrap();
            buffer_text(&terminal)
        };

        let hidden = draw(&mut process);
        assert!(!hidden.contains("Keys"), "{hidden}");
        assert!(!hidden.contains("refresh"), "{hidden}");

        assert!(process.handle_navi_event(&key(KeyCode::Char('?'))));
        let shown = draw(&mut process);
        assert!(shown.contains("Keys"), "{shown}");
        assert!(shown.contains("refresh"), "{shown}");
        assert!(shown.contains("SIGTERM"), "{shown}");
        let keys_row = shown.lines().position(|line| line.contains("Keys"));
        let refresh_row = shown.lines().position(|line| line.contains("refresh"));
        assert!(keys_row.is_some_and(|row| row > 0), "{shown}");
        assert!(
            refresh_row.is_some_and(|row| row + 1 < shown.lines().count()),
            "{shown}"
        );

        assert!(process.handle_navi_event(&key(KeyCode::Esc)));
        assert!(process.detail.is_some());
        let hidden_again = draw(&mut process);
        assert!(!hidden_again.contains("Keys"), "{hidden_again}");
        assert!(!hidden_again.contains("refresh"), "{hidden_again}");
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

    #[test]
    fn process_list_shows_keys_only_after_question_mark() {
        let mut process = process_with_items(vec![process_item(1), process_item(2)]);

        let draw = |process: &mut ResProcess| {
            let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
            terminal
                .draw(|frame| {
                    process.render_process_list(
                        frame,
                        &DetailArg {
                            rect: Rect::new(0, 0, 80, 24),
                            active: true,
                        },
                    );
                })
                .unwrap();
            buffer_text(&terminal)
        };

        let hidden = draw(&mut process);
        assert!(!hidden.contains("Keys"), "{hidden}");
        assert!(!hidden.contains("sort by PID"), "{hidden}");

        assert!(process.handle_navi_event(&key(KeyCode::Char('?'))));
        assert!(process.list_keys_hint);
        assert!(!process.detail_keys_hint);
        let shown = draw(&mut process);
        assert!(shown.contains("Keys"), "{shown}");
        assert!(shown.contains("sort by PID"), "{shown}");
        assert!(shown.contains("filter"), "{shown}");
        let keys_row = shown.lines().position(|line| line.contains("Keys"));
        assert!(keys_row.is_some_and(|row| row > 0), "{shown}");

        assert!(process.handle_navi_event(&key(KeyCode::Esc)));
        assert!(!process.list_keys_hint);
        let hidden_again = draw(&mut process);
        assert!(!hidden_again.contains("sort by PID"), "{hidden_again}");
    }

    #[test]
    fn question_mark_follows_the_focused_process_pane() {
        let mut process = process_with_items(vec![process_item(7)]);
        process.set_detail(process_item(7));
        process.set_process_focus(ProcessPanelFocus::List);

        assert!(process.handle_navi_event(&key(KeyCode::Char('?'))));
        assert!(process.list_keys_hint);
        assert!(!process.detail_keys_hint);

        assert!(process.handle_navi_event(&key(KeyCode::Esc)));
        process.set_process_focus(ProcessPanelFocus::Detail);
        assert!(process.handle_navi_event(&key(KeyCode::Char('?'))));
        assert!(!process.list_keys_hint);
        assert!(process.detail_keys_hint);
    }

    #[test]
    fn filter_consumes_question_mark() {
        let mut process = process_with_items(vec![process_item(1)]);
        assert!(process.handle_navi_event(&key(KeyCode::Char('/'))));
        assert!(process.handle_navi_event(&key(KeyCode::Char('?'))));
        assert!(!process.list_keys_hint);
        assert!(process.filter.as_ref().unwrap().get_input().contains('?'));
    }
}
