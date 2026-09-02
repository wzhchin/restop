use chin_tools::AResult;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Style, Stylize},
    symbols::line::*,
    text::{Line, Span},
    widgets::Widget,
};

use crate::view::theme::{SharedTheme, Theme};

use super::{ls_common, ls_kv, s_label};

#[derive(Clone, Copy, Debug)]
struct Border {
    top_left: &'static str,
    top_right: &'static str,
    top_horizontal: &'static str,
    horizontal: &'static str,
    vertical: &'static str,
    bottom_left: &'static str,
    bottom_right: &'static str,
    style: Style,
}

impl Border {
    fn new(focused: bool) -> Self {
        Self::styled(focused, Theme::default().border(focused))
    }

    fn styled(focused: bool, style: Style) -> Self {
        macro_rules! fcs {
            ($when_focused:expr, $when_unfocused:expr) => {
                if focused {
                    $when_focused
                } else {
                    $when_unfocused
                }
            };
        }

        Self {
            top_left: fcs!(DOUBLE_TOP_LEFT, TOP_LEFT),
            top_right: fcs!(DOUBLE_TOP_RIGHT, TOP_RIGHT),
            top_horizontal: fcs!(DOUBLE_HORIZONTAL, HORIZONTAL),
            horizontal: fcs!(DOUBLE_HORIZONTAL, HORIZONTAL),
            vertical: fcs!(DOUBLE_VERTICAL, VERTICAL),
            bottom_left: fcs!(DOUBLE_BOTTOM_LEFT, BOTTOM_LEFT),
            bottom_right: fcs!(DOUBLE_BOTTOM_RIGHT, BOTTOM_RIGHT),
            style,
        }
    }

    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let top = area.top();
        let right = area.right().saturating_sub(1);
        let bottom = area.bottom().saturating_sub(1);
        let left = area.left();
        let style = self.style;

        buf.set_string(left, top, self.top_left, style);
        buf.set_string(right, top, self.top_right, style);
        buf.set_string(left, bottom, self.bottom_left, style);
        buf.set_string(right, bottom, self.bottom_right, style);

        for x in left.saturating_add(1)..right {
            buf.set_string(x, top, self.top_horizontal, style);
            buf.set_string(x, bottom, self.horizontal, style);
        }
        for y in top.saturating_add(1)..bottom {
            buf.set_string(left, y, self.vertical, style);
            buf.set_string(right, y, self.vertical, style);
        }
    }
}

pub fn render_border(focused: bool, area: Rect, buf: &mut Buffer) {
    Border::new(focused).render(area, buf);
}

#[derive(Clone, Debug)]
pub struct GroupedLines<'a> {
    width: u16,
    start: Option<u16>,
    end: Option<u16>,
    lines: Vec<Line<'a>>,
    title: String,
    theme: SharedTheme,
    focused: bool,
    active: bool,
}

impl<'a> GroupedLines<'a> {
    pub fn new<T>(title: T, width: u16, theme: &SharedTheme) -> Self
    where
        T: Into<String>,
    {
        Self {
            start: None,
            end: None,
            lines: vec![],
            title: title.into(),
            width,
            theme: theme.clone(),
            focused: false,
            active: false,
        }
    }

    pub fn focused(self, focused: bool) -> Self {
        Self { focused, ..self }
    }
    pub fn active(self, active: bool) -> Self {
        Self { active, ..self }
    }

    pub fn start(self, start: Option<u16>) -> Self {
        Self { start, ..self }
    }

    pub fn end(self, end: Option<u16>) -> Self {
        Self { end, ..self }
    }

    pub fn inner_width(&self) -> u16 {
        self.width.saturating_sub(2)
    }

    pub fn lines<F>(self, builder: F) -> AResult<Self>
    where
        F: FnOnce(u16) -> AResult<Vec<Line<'a>>>,
    {
        let lines = builder(self.inner_width())?;
        Ok(Self { lines, ..self })
    }

    pub fn height(&self) -> usize {
        self.lines.len() + 2
    }

    pub fn builder(width: u16, theme: &SharedTheme) -> GroupedLinesBuilder {
        GroupedLinesBuilder::new(width, theme)
    }
}

impl<'a> Widget for GroupedLines<'a> {
    fn render(self, area: ratatui::prelude::Rect, buf: &mut ratatui::prelude::Buffer)
    where
        Self: Sized,
    {
        let start = self.start.unwrap_or(0);
        let end = self.end.unwrap_or(u16::MAX);

        let offset = start;
        let border = Border::styled(self.focused, self.theme.border(self.focused));
        let border_style = border.style;

        for i in start..end {
            if i.saturating_sub(start) > area.height {
                break;
            }

            let y = (area.y + i).saturating_sub(offset);

            if i == 0 {
                let mut s = vec![];
                s.push(Span::styled(border.top_left, border_style));
                s.push(Span::raw(" "));

                let mut title = Span::from(self.title.as_str());
                if self.focused || self.active {
                    title = title.bold();
                }

                s.push(title);

                s.push(" ".into());
                for _ in 0..(area
                    .width
                    .saturating_sub(4)
                    .saturating_sub(self.title.len() as u16))
                {
                    s.push(Span::styled(border.top_horizontal, border_style));
                }
                s.push(Span::styled(border.top_right, border_style));

                Line::from(s).render(
                    Rect {
                        x: area.x,
                        y,
                        width: area.width,
                        height: 1,
                    },
                    buf,
                );
            } else if i.saturating_sub(1) as usize >= self.lines.len() {
                let mut s = String::new();
                s.push_str(border.bottom_left);
                for _ in 0..(area.width.saturating_sub(2)) {
                    s.push_str(border.horizontal);
                }
                s.push_str(border.bottom_right);

                Line::styled(s, border_style).render(
                    Rect {
                        x: area.x,
                        y,
                        width: area.width,
                        height: 1,
                    },
                    buf,
                );

                break;
            } else {
                Span::styled(border.vertical, border_style).render(
                    Rect {
                        x: area.x,
                        y,
                        width: 1,
                        height: 1,
                    },
                    buf,
                );
                let line = self.lines[(i.saturating_sub(1)) as usize].clone();

                line.render(
                    Rect {
                        x: area.x.saturating_add(1),
                        y,
                        width: area.width.saturating_sub(2),
                        height: 1,
                    },
                    buf,
                );

                Span::styled(border.vertical, border_style).render(
                    Rect {
                        x: area.right().saturating_sub(1),
                        y,
                        width: 1,
                        height: 1,
                    },
                    buf,
                );
            }
        }
    }
}

enum GroupedLinesBuilderType {
    KV(String, String),
    Value(String),
    EmptySep,
    Line(Line<'static>),
    Lines(Vec<Line<'static>>),
    MultiKVSingleLine(Vec<(String, String)>),
}

pub struct GroupedLinesBuilder {
    width: u16,
    pairs: Vec<GroupedLinesBuilderType>,
    theme: SharedTheme,
    sep: bool,
    focused: bool,
    active: bool,
}

impl GroupedLinesBuilder {
    fn new(width: u16, theme: &SharedTheme) -> Self {
        Self {
            width,
            pairs: vec![],
            theme: theme.to_owned(),
            sep: false,
            focused: false,
            active: false,
        }
    }

    pub fn multi_kv_single_line<T>(self, kvs: Vec<(&str, T)>) -> Self
    where
        T: Into<String>,
    {
        let mut s = if self.sep { self.empty_sep() } else { self };
        let kvs = kvs
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.into()))
            .collect();
        s.pairs
            .push(GroupedLinesBuilderType::MultiKVSingleLine(kvs));
        s
    }

    pub fn active(self, active: bool) -> Self {
        Self { active, ..self }
    }

    pub fn kv_sep<T>(self, key: &str, value: T) -> Self
    where
        T: Into<String>,
    {
        let mut s = self.kv(key, value);
        s.sep = true;
        s
    }

    pub fn kv<T>(self, key: &str, value: T) -> Self
    where
        T: Into<String>,
    {
        let mut s = if self.sep { self.empty_sep() } else { self };
        s.pairs
            .push(GroupedLinesBuilderType::KV(key.to_string(), value.into()));
        s
    }

    pub fn value<T>(self, value: T) -> Self
    where
        T: Into<String>,
    {
        let mut s = if self.sep { self.empty_sep() } else { self };
        s.pairs.push(GroupedLinesBuilderType::Value(value.into()));
        s
    }

    pub fn line(mut self, line: Line<'static>) -> Self {
        self.pairs.push(GroupedLinesBuilderType::Line(line));
        self
    }

    pub fn lines(mut self, lines: Vec<Line<'static>>) -> Self {
        self.pairs.push(GroupedLinesBuilderType::Lines(lines));
        self
    }

    pub fn empty_sep(mut self) -> Self {
        self.pairs.push(GroupedLinesBuilderType::EmptySep);
        self
    }

    pub fn build<T>(self, title: T) -> AResult<GroupedLines<'static>>
    where
        T: Into<String>,
    {
        GroupedLines::new(title.into(), self.width, &self.theme)
            .focused(self.focused)
            .active(self.active)
            .lines(|width| {
                let mut lines = vec![];
                let key_style = self.theme.key(self.active);
                let value_style = self.theme.value(self.active);

                for p in self.pairs {
                    match p {
                        GroupedLinesBuilderType::KV(k, v) => {
                            lines.extend(ls_kv(Some(&k), &v, width, key_style, value_style));
                        }
                        GroupedLinesBuilderType::Line(line) => lines.push(line),
                        GroupedLinesBuilderType::Lines(ls) => lines.extend(ls),
                        GroupedLinesBuilderType::EmptySep => lines.push(Line::raw("")),
                        GroupedLinesBuilderType::Value(v) => lines.extend(ls_common(&v, width)),
                        GroupedLinesBuilderType::MultiKVSingleLine(kvs) => {
                            let mut spans = vec![];
                            let mut visit = false;
                            for (k, v) in kvs {
                                if visit {
                                    spans.push(Span::raw(" - "))
                                } else {
                                    visit = true;
                                }
                                spans.push(s_label(&k, key_style));
                                spans.push(Span::raw(" "));
                                spans.push(s_label(&v, value_style));
                            }

                            lines.push(spans.into());
                        }
                    }
                }

                Ok(lines)
            })
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{buffer::Buffer, layout::Rect, style::Color, widgets::Widget};

    use super::{render_border, GroupedLines};
    use crate::view::theme::{SharedTheme, Theme};

    fn cell_fg(buf: &Buffer, x: u16, y: u16) -> Color {
        buf[(x, y)].style().fg.unwrap_or(Color::Reset)
    }

    #[test]
    fn unfocused_render_border_is_gray() {
        let area = Rect::new(0, 0, 6, 3);
        let mut buf = Buffer::empty(area);
        render_border(false, area, &mut buf);
        assert_eq!(cell_fg(&buf, 0, 0), Color::Gray);
        assert_eq!(cell_fg(&buf, 5, 2), Color::Gray);
    }

    #[test]
    fn focused_render_border_keeps_default_fg() {
        let area = Rect::new(0, 0, 6, 3);
        let mut buf = Buffer::empty(area);
        render_border(true, area, &mut buf);
        assert_eq!(cell_fg(&buf, 0, 0), Color::Reset);
    }

    #[test]
    fn unfocused_grouped_lines_border_is_gray() {
        let theme = SharedTheme::new(Theme::default());
        let widget = GroupedLines::builder(8, &theme)
            .value("x")
            .build("T")
            .unwrap();
        let area = Rect::new(0, 0, 8, 3);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);
        assert_eq!(cell_fg(&buf, 0, 0), Color::Gray);
        assert_eq!(cell_fg(&buf, 0, 1), Color::Gray);
        assert_eq!(cell_fg(&buf, 0, 2), Color::Gray);
    }

    #[test]
    fn focused_grouped_lines_border_keeps_default_fg() {
        let theme = SharedTheme::new(Theme::default());
        let widget = GroupedLines::builder(8, &theme)
            .value("x")
            .build("T")
            .unwrap()
            .focused(true);
        let area = Rect::new(0, 0, 8, 3);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);
        assert_eq!(cell_fg(&buf, 0, 0), Color::Reset);
        assert_eq!(cell_fg(&buf, 0, 1), Color::Reset);
        assert_eq!(cell_fg(&buf, 0, 2), Color::Reset);
    }
}
