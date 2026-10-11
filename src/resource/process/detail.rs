use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::Line,
    widgets::{Block, BorderType, Borders},
};

use crate::{
    component::ls_kv,
    sensor::{
        process::{load_process_details, ProcessAction, ProcessItem},
        process_data::Containerization,
        units::{convert_seconds, convert_speed, convert_storage},
    },
    tarits::format_fraction_as_percent,
    view::DetailArg,
};

#[derive(Debug)]
pub(super) struct ProcessDetail {
    pub(super) item: ProcessItem,
    pub(super) executable: String,
    pub(super) workdir: String,
    pub(super) environment: Vec<String>,
    pub(super) environment_truncated: bool,
}

impl ProcessDetail {
    pub(super) fn load(item: ProcessItem) -> Self {
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

const DETAIL_KEY_HINTS: &[(&str, &str)] = &[
    ("↑/↓", "scroll / select"),
    ("PgUp/PgDn", "page"),
    ("r", "refresh"),
    ("t", "SIGTERM"),
    ("i", "SIGINT"),
    ("h", "SIGHUP"),
    ("k", "SIGKILL"),
    ("s", "SIGSTOP"),
    ("c", "SIGCONT"),
    ("←", "focus list / close"),
    ("→", "focus detail"),
    ("Esc", "close"),
    ("?", "hide this popup"),
];

pub(super) fn process_action_name(action: ProcessAction) -> &'static str {
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

impl super::ResProcess {
    fn rebuild_detail_lines(&mut self, width: u16) {
        if self.detail_line_width == width && !self.detail_lines.is_empty() {
            return;
        }
        let Some(detail) = self.detail.as_ref() else {
            return;
        };
        let key_style = self.theme.key(true);
        let value_style = self.theme.value(true);
        let mut lines = Vec::new();
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
            .map(|detail| format!(" Process {} ", detail.item.pid))
            .unwrap_or_else(|| " Process ".to_owned());
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
        if self.detail_keys_hint {
            super::render_keys_popup(frame, rect, &self.theme, focused, DETAIL_KEY_HINTS);
        }
    }
    pub(crate) fn render_process_detail(&mut self, frame: &mut ratatui::Frame, args: &DetailArg) {
        self.render_detail_drawer(frame, args.rect, self.process_detail_focused());
    }
}
