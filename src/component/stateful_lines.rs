use std::{
    ops::{Deref, DerefMut},
    sync::Arc,
};

use ratatui::{layout::Rect, text::Line, Frame};

use super::grouped_lines::GroupedLines;

#[derive(Default, Clone, Debug)]
pub struct LinesVerticalState {
    pub show_start: usize,
    pub view_height: u16,
    cur_line: usize,
    end: usize,
    dirty: bool,
}

impl LinesVerticalState {
    pub fn focused_index(&self) -> Option<usize> {
        (self.cur_line < self.end).then_some(self.cur_line)
    }

    pub fn show_end(&self) -> usize {
        self.show_start.saturating_add(self.view_height.into())
    }

    pub fn focus_next(&mut self) {
        let previous = (self.cur_line, self.show_start);
        self.cur_line = self
            .cur_line
            .saturating_add(1)
            .clamp(0, self.end.saturating_sub(1));
        if self.cur_line >= self.show_end().saturating_sub(3) {
            self.show_start = self
                .show_start
                .saturating_add(1)
                .clamp(0, self.end.saturating_sub(self.view_height.into()));
        }
        if previous != (self.cur_line, self.show_start) {
            self.dirty = true;
        }
    }

    pub fn focus_prev(&mut self) {
        let previous = (self.cur_line, self.show_start);
        self.cur_line = self.cur_line.saturating_sub(1).clamp(0, self.end);
        if self.cur_line >= self.show_start.saturating_add(3) {
            self.show_start = self
                .show_start
                .saturating_sub(1)
                .clamp(0, self.end.saturating_sub(self.view_height.into()));
        }
        if previous != (self.cur_line, self.show_start) {
            self.dirty = true;
        }
    }

    fn update_end(&mut self, end: usize) {
        self.end = end;
        self.cur_line = self.cur_line.min(self.end.saturating_sub(1));
        self.show_start = self
            .show_start
            .clamp(0, self.end.saturating_sub(self.view_height.into()));
    }

    pub fn update_view_height(&mut self, view_height: u16) {
        let previous = (self.view_height, self.show_start);
        self.view_height = view_height;
        self.show_start = self.show_start.clamp(
            self.cur_line.saturating_sub(self.view_height.into()),
            self.cur_line,
        );
        if previous != (self.view_height, self.show_start) {
            self.dirty = true;
        }
    }
}

#[derive(Clone, Debug)]
pub struct StatefulColumn<'a> {
    lines: Vec<Line<'a>>,
    state: LinesVerticalState,
    header: Option<Line<'a>>,
}

impl<'a> Deref for StatefulColumn<'a> {
    type Target = LinesVerticalState;

    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl<'a> DerefMut for StatefulColumn<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}

impl<'a> Default for StatefulColumn<'a> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> StatefulColumn<'a> {
    pub fn new() -> Self {
        Self {
            lines: vec![],
            state: Default::default(),
            header: None,
        }
    }

    pub fn set_header(&mut self, header: Line<'static>) {
        self.header.replace(header);
    }

    pub fn is_dirty(&self) -> bool {
        self.state.dirty
    }

    pub fn mark_dirty(&mut self) {
        self.state.dirty = true;
    }

    pub fn update_lines<T, F>(&mut self, eles: &Arc<Vec<T>>, convert: F)
    where
        F: Fn(&T, bool) -> Line<'a>,
    {
        self.update_end(eles.len());
        let lines: Vec<Line<'a>> = eles
            .iter()
            .enumerate()
            .skip(self.show_start)
            .take(self.view_height.saturating_sub(1).into())
            .map(|(id, e)| convert(e, self.cur_line == id))
            .collect();

        self.lines = lines;
        self.state.dirty = false;
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        frame.render_widget(&self.header, Rect { height: 1, ..area });

        // Keep formatted lines alive across redraws and render them by reference.
        for (offset, line) in self.lines.iter().enumerate() {
            let offset = u16::try_from(offset).unwrap_or(u16::MAX);
            if offset >= area.height.saturating_sub(1) {
                break;
            }
            frame.render_widget(
                line,
                Rect {
                    y: area.y.saturating_add(1).saturating_add(offset),
                    height: 1,
                    ..area
                },
            );
        }
    }
}

#[derive(Default, Debug, Clone)]
pub struct StatefulGroupedLines<'a> {
    pub show_start: usize,
    view_height: u16,
    cur_line: usize,
    blocks: Vec<(IndexedRange, GroupedLines<'a>)>,
}

#[derive(Clone, Debug, Copy)]
pub struct IndexedRange {
    pub index: usize,
    pub start: usize,
    pub end: usize,
}

impl<'a> StatefulGroupedLines<'a> {
    pub fn show_end(&self) -> usize {
        self.show_start.saturating_add(self.view_height.into())
    }

    pub fn focused(&self) -> Option<IndexedRange> {
        self.blocks
            .iter()
            .find(|(r, _)| r.end > self.cur_line && r.start <= self.cur_line)
            .map(|(r, _)| *r)
    }

    pub fn focused_index(&self) -> Option<usize> {
        self.focused().as_ref().map(|e| e.index)
    }

    pub fn _update_view_height(&mut self, view_height: u16) {
        self.view_height = view_height;
        self.show_start = self.show_start.clamp(
            self.cur_line.saturating_sub(self.view_height.into()),
            self.cur_line,
        )
    }

    pub fn focus_next(&mut self) {
        if let Some(range) = self.focused() {
            if range.end > self.show_end() {
                self.show_start = self.show_start.saturating_add(1);
                self.cur_line = self.cur_line.saturating_add(1);
            } else {
                let next = self.blocks.get(range.index.saturating_add(1));
                if let Some((next, _)) = next {
                    self.cur_line = next.start;
                    if next.end >= self.show_end() {
                        self.show_start = next.end.saturating_sub(self.view_height.into());
                    }
                }
            }
        }
        if let Some((last, _)) = self.blocks.last() {
            self.show_start = std::cmp::min(
                self.show_start,
                last.end.saturating_sub(self.view_height.into()),
            );
            self.cur_line = std::cmp::min(self.cur_line, last.end - 1);
        }
    }

    pub fn focus_prev(&mut self) {
        if let Some(range) = self.focused() {
            if range.end <= self.show_start {
                self.show_start = self.show_start.saturating_sub(1);
                self.cur_line = self.cur_line.saturating_sub(1);
            } else {
                let previous = self.blocks.get(range.index.saturating_sub(1));
                if let Some((previous, _)) = previous {
                    if previous.start < self.show_end() {
                        self.cur_line = previous.start;
                    }
                    if previous.start <= self.show_start {
                        self.show_start = previous.start.saturating_sub(self.view_height.into());
                    }
                }
            }
        }
    }

    pub fn update_blocks(&mut self, blocks: Vec<GroupedLines<'a>>) {
        let mut visited: usize = 0;
        let mut new_blocks = vec![];
        for (index, block) in blocks.into_iter().enumerate() {
            let end = visited.saturating_add(block.height());
            let pos = IndexedRange {
                index,
                start: visited,
                end,
            };
            visited = end;
            new_blocks.push((pos, block));
        }

        self.blocks = new_blocks;
    }

    pub fn render(&mut self, frame: &mut Frame, rect: Rect, active: bool) {
        if rect.width == 0 || rect.height == 0 {
            return;
        }
        self._update_view_height(rect.height);

        let show_end = self.show_end();
        let show_start = self.show_start;

        for (range, block) in self.blocks.iter() {
            if !(range.end < show_start || range.start > show_end) {
                let rect_y;
                let mut start_y = None;
                if show_start > range.start {
                    rect_y = 0;
                    start_y.replace(show_start.saturating_sub(range.start) as u16);
                } else {
                    rect_y = range.start.saturating_sub(show_start);
                };

                let mut end_y = None;
                let height;
                if show_end >= range.end {
                    height = range.end - range.start;
                } else {
                    height = show_end.saturating_sub(range.start);
                    end_y.replace(show_end.saturating_sub(range.start) as u16);
                };

                let focused = self.focused_index() == Some(range.index);
                let block = block
                    .clone()
                    .start(start_y)
                    .end(end_y)
                    .focused(focused && active)
                    .active(focused);

                let y = rect
                    .y
                    .saturating_add(rect_y as u16)
                    .clamp(rect.y, rect.y.saturating_add(rect.height).saturating_sub(1));

                let rect = Rect {
                    x: rect.x,
                    y,
                    width: rect.width,
                    height: height as u16,
                };

                frame.render_widget(block, rect);
            }
        }
    }
}

pub enum StatefulLinesType<'a, 'b> {
    Groups(&'b mut StatefulGroupedLines<'a>),
    Lines(&'b mut StatefulColumn<'a>),
}

impl<'a, 'b> StatefulLinesType<'a, 'b> {
    pub fn focus_next(self) {
        match self {
            StatefulLinesType::Groups(ls) => ls.focus_next(),
            StatefulLinesType::Lines(virt) => virt.focus_next(),
        }
    }

    pub fn focus_prev(self) {
        match self {
            StatefulLinesType::Groups(ls) => ls.focus_prev(),
            StatefulLinesType::Lines(virt) => virt.focus_prev(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::LinesVerticalState;

    #[test]
    fn focus_next_stays_on_last_line() {
        let mut state = LinesVerticalState {
            view_height: 10,
            ..Default::default()
        };
        state.update_end(2);

        state.focus_next();
        state.focus_next();

        assert_eq!(state.focused_index(), Some(1));
    }

    #[test]
    fn updating_line_count_clamps_removed_selection() {
        let mut state = LinesVerticalState {
            view_height: 10,
            ..Default::default()
        };
        state.update_end(3);
        state.focus_next();
        state.focus_next();
        state.update_end(1);
        assert_eq!(state.focused_index(), Some(0));

        state.update_end(0);
        assert_eq!(state.focused_index(), None);
    }
}
