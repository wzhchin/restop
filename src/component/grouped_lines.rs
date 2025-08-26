use chin_tools::AResult;
use ratatui::{
    layout::Rect,
    style::Stylize,
    symbols::line::*,
    text::{Line, Span},
    widgets::Widget,
};

use crate::view::theme::SharedTheme;

use super::{ls_common, ls_kv, s_label};

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

        let fg = self.theme.fg();

        let offset = start;

        let tl = if self.focused { "╒" } else { TOP_LEFT };
        let tr = if self.focused { "╕" } else { TOP_RIGHT };
        let thor = if self.focused {
            DOUBLE_HORIZONTAL
        } else {
            HORIZONTAL
        };
        let hor = HORIZONTAL;
        let ver = VERTICAL;
        let bl = BOTTOM_LEFT;
        let br = BOTTOM_RIGHT;

        for i in start..end {
            if i.saturating_sub(start) > area.height {
                break;
            }

            let y = (area.y + i).saturating_sub(offset);

            if i == 0 {
                let mut s = vec![];
                s.push(Span::raw(tl));
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
                    s.push(thor.into());
                }
                s.push(tr.into());

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
                s.push_str(bl);
                for _ in 0..(area.width.saturating_sub(2)) {
                    s.push_str(hor);
                }
                s.push_str(br);

                Line::styled(s, fg).render(
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
                Span::styled(ver, fg).render(
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

                Span::styled(ver, fg).render(
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
