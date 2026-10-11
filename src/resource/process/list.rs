use std::{
    cmp::Ordering,
    sync::{Arc, RwLock},
};

use once_cell::sync::Lazy;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
};
use unicode_width::UnicodeWidthChar;

use crate::{
    component::{input::Input, render_border, s_label, stateful_lines::StatefulLinesType},
    resource::Resource,
    sensor::{process::ProcessItem, units::conver_storage_width4},
    tarits::format_percent_number,
    view::DetailArg,
};

static PROCESS_SORT_TYPE: Lazy<RwLock<Option<(ProcessCell, bool)>>> =
    Lazy::new(|| RwLock::new(None));

const LIST_KEY_HINTS: &[(&str, &str)] = &[
    ("↑/↓", "select"),
    ("→", "open detail"),
    ("←", "close detail"),
    ("/", "filter"),
    ("Ctrl-S", "filter"),
    ("Esc", "close filter or detail"),
    ("Alt-p", "sort by PID"),
    ("Alt-c", "sort by CPU"),
    ("Alt-m", "sort by memory"),
    ("Alt-n", "sort by command"),
    ("?", "hide this popup"),
];

fn get_process_sort() -> Option<(ProcessCell, bool)> {
    *PROCESS_SORT_TYPE.read().unwrap()
}

pub(super) fn try_change_sort(c: ProcessCell) {
    let sort = get_process_sort();

    if let Ok(mut write) = PROCESS_SORT_TYPE.write() {
        if let Some((cell, desc)) = sort {
            write.replace((c, if cell == c { !desc } else { true }));
        } else {
            write.replace((c, true));
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(dead_code)]
#[allow(clippy::upper_case_acronyms)]
pub(super) enum ProcessCell {
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
pub(super) struct LineBuilder {
    labels: Vec<ProcessCell>,
}

impl LineBuilder {
    pub(super) fn new() -> Self {
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

    pub(super) fn to_line(&self, item: &ProcessItem, active: bool) -> Line<'static> {
        let spans: Vec<Span<'static>> = self.labels.iter().map(|pc| pc.to_value(item)).collect();
        let line: Line<'static> = spans.into();
        if active {
            line.add_modifier(Modifier::REVERSED)
        } else {
            line
        }
    }

    pub(super) fn to_header(&self) -> Line<'static> {
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

impl super::ResProcess {
    pub(super) fn rebuild_visible_data(&mut self) {
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
        if self.list_keys_hint {
            super::render_keys_popup(
                frame,
                args.rect,
                &self.theme,
                self.process_list_focused(),
                LIST_KEY_HINTS,
            );
        }
    }
}
