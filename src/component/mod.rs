use std::ops::Add;

use ratatui::{
    style::{Color, Modifier, Stylize},
    text::{Line, Span},
};

use crate::ring::Ring;
use chin_tools::utils::string_util::split_by_len;
use ratatui::style::Style;

pub mod grouped_lines;
pub mod input;
pub mod stateful_lines;

pub use grouped_lines::render_border;

pub fn ls_kv(
    key: Option<&str>,
    value: &str,
    width: u16,
    key_style: Style,
    value_style: Style,
) -> Vec<Line<'static>> {
    if width == 0 {
        return vec![];
    }

    if let Some(key) = key {
        if key.len() + value.len() + 1 < width as usize {
            vec![vec![
                s_label(key, key_style),
                s_label(" ", value_style),
                s_value(value, value_style),
            ]
            .into()]
        } else {
            let mut lines = vec![];

            split_by_len(key, width as usize)
                .iter()
                .for_each(|e| lines.push(Line::from(s_label(e, key_style))));

            split_by_len(value, width as usize)
                .iter()
                .for_each(|e| lines.push(Line::from(s_value(e, value_style))));

            lines
        }
    } else if value.len() < width as usize {
        vec![vec![s_value(value, value_style)].into()]
    } else {
        let mut lines = vec![];

        split_by_len(value, width as usize)
            .iter()
            .for_each(|e| lines.push(Line::from(s_value(e, value_style))));

        lines
    }
}

pub fn s_percent_graph(
    value: f64,
    total: f64,
    width: u16,
    high_is_good: bool,
) -> Vec<Span<'static>> {
    let percent = (value * 100. / total) as u16;
    let graph_width = width.saturating_sub(3);

    let mut colors = [
        Color::Black,
        Color::Black,
        Color::Black,
        Color::Black,
        Color::Black,
        Color::Black,
    ];

    if high_is_good {
        colors.reverse();
    }

    let value_width = graph_width as usize * percent as usize / 100;
    let color = percent / (100 / colors.len() as u16);

    let color = if color == 0 {
        colors[0]
    } else if color as usize >= colors.len() {
        *colors.last().unwrap()
    } else {
        colors[color as usize]
    };

    let legent_width = if percent < 10 {
        1
    } else if percent < 100 {
        2
    } else {
        3
    };

    let value_width = std::cmp::min(value_width as u16, graph_width.saturating_sub(legent_width));
    let padding_width = graph_width
        .saturating_sub(legent_width)
        .saturating_sub(value_width);

    let mut graph = String::new();

    for _ in 0..value_width {
        graph.push('|');
    }

    for _ in 0..padding_width {
        graph.push(' ');
    }

    vec![
        Span::raw("["),
        Span::styled(graph, Style::new().fg(color)),
        Span::raw(format!("{}%", percent)),
        Span::raw("]"),
    ]
}

/// Map a history sample onto discrete braille bar units.
/// A zero or non-finite value span yields 0 (no division by zero).
pub(crate) fn history_bar_units(
    value: f64,
    min_value: f64,
    max_value: f64,
    line_height: u16,
) -> usize {
    const MAX_HEIGHT: usize = 4;
    let steps = (MAX_HEIGHT as i32).saturating_mul(line_height as i32) as f64;
    let span = max_value - min_value;
    if steps == 0.0 || !span.is_finite() || span == 0.0 {
        return 0;
    }
    let bar_sep = span / steps;
    if !bar_sep.is_finite() || bar_sep == 0.0 {
        return 0;
    }
    let units = ((value - min_value) / bar_sep).round();
    if !units.is_finite() || units <= 0.0 {
        0
    } else {
        units as usize
    }
}

pub fn s_history_graph(
    width: u16,
    ring: &Ring<f64>,
    max_value: f64,
    min_value: f64,
    line_height: u16,
    color: Color,
) -> Vec<Span<'static>> {
    const MAX_HEIGHT: usize = 4;

    let bars = [
        [' ', '⢀', '⢠', '⢰', '⢸'],
        ['⡀', '⣀', '⣠', '⣰', '⣸'],
        ['⡄', '⣄', '⣤', '⣴', '⣼'],
        ['⡆', '⣆', '⣦', '⣶', '⣾'],
        ['⡇', '⣇', '⣧', '⣷', '⣿'],
    ];

    let mut lines: Vec<String> = vec![];
    for _ in 0..line_height {
        lines.push(String::with_capacity(width.into()));
    }

    let values: Vec<&f64> = ring.new_to_old_iter().take(width as usize * 2).collect();
    let true_len = values.len().div_ceil(2);
    let pad = (width as usize).saturating_sub(true_len);

    // Build left→right (oldest→newest of the window) with push, reverse once.
    // Avoids O(n²) insert(0) per column.
    values.chunks(2).rev().for_each(|e| {
        // chunks are newest-first pairs; reverse iteration yields oldest-first columns.
        let left = e
            .first()
            .map(|e| history_bar_units(**e, min_value, max_value, line_height))
            .unwrap_or(0);
        let right = e
            .get(1)
            .map(|e| history_bar_units(**e, min_value, max_value, line_height))
            .unwrap_or(0);

        for i in 1..=line_height {
            let max = i as usize * MAX_HEIGHT;
            let r = if max <= left {
                MAX_HEIGHT
            } else {
                (left + MAX_HEIGHT).saturating_sub(max)
            };
            let l = if max <= right {
                MAX_HEIGHT
            } else {
                (right + MAX_HEIGHT).saturating_sub(max)
            };

            if let Some(line) = lines.get_mut(i as usize - 1) {
                let mut sym = bars.get(l).and_then(|v| v.get(r)).unwrap_or(&'?');
                if i == 1 && line_height > 1 {
                    sym = bars
                        .get(l.add(1).clamp(1, MAX_HEIGHT))
                        .and_then(|v| v.get(r.add(1).clamp(1, MAX_HEIGHT)))
                        .unwrap_or(&'?');
                }
                line.push(*sym);
            }
        }
    });

    // Prepend padding (oldest side) then reverse each line so newest is on the right.
    if pad > 0 {
        let limit = if line_height > 1 { 1 } else { 0 };
        for (idx, line) in lines.iter_mut().enumerate() {
            let fill = if idx < limit { '⣀' } else { ' ' };
            let mut padded = String::with_capacity(width as usize);
            for _ in 0..pad {
                padded.push(fill);
            }
            padded.push_str(line);
            *line = padded;
        }
    }

    // Graph is drawn bottom-up (line 0 = lowest bar row).
    lines
        .into_iter()
        .rev()
        .map(|e| Span::from(e).fg(color))
        .collect()
}

pub fn ls_history_graph(
    width: u16,
    ring: &Ring<f64>,
    max_value: f64,
    min_value: f64,
    line_height: u16,
    color: Color,
) -> Vec<Line<'static>> {
    s_history_graph(width, ring, max_value, min_value, line_height, color)
        .into_iter()
        .map(Line::from)
        .collect()
}

pub fn s_label(label: &str, style: Style) -> Span<'static> {
    Span::styled(String::from(label), style)
}

pub fn s_value(label: &str, style: Style) -> Span<'static> {
    Span::styled(String::from(label), style)
}

pub fn ls_italic(label: &str, width: u16) -> Vec<Line<'static>> {
    ls_kv(
        None,
        label,
        width,
        Style::new(),
        Style::new().add_modifier(Modifier::ITALIC),
    )
}

pub fn ls_common(label: &str, width: u16) -> Vec<Line<'static>> {
    ls_kv(None, label, width, Style::new(), Style::new())
}

pub fn ls_style(label: &str, width: u16, style: Style) -> Vec<Line<'static>> {
    ls_kv(None, label, width, Style::new(), style)
}

pub trait PaddingH {
    fn padding(self) -> Self;
}

impl PaddingH for Vec<Span<'static>> {
    fn padding(self) -> Self {
        let mut this = self;
        this.insert(0, Span::raw(" "));
        this.push(Span::raw(" "));
        this
    }
}

#[cfg(test)]
mod tests {
    use super::{history_bar_units, s_history_graph};
    use crate::ring::Ring;
    use ratatui::style::Color;

    #[test]
    fn zero_span_history_does_not_divide_by_zero() {
        assert_eq!(history_bar_units(0.0, 0.0, 0.0, 3), 0);
        assert_eq!(history_bar_units(50.0, 50.0, 50.0, 3), 0);
        assert_eq!(history_bar_units(f64::NAN, 0.0, 0.0, 3), 0);
        assert!(history_bar_units(50.0, 0.0, 100.0, 1) > 0);

        let mut ring = Ring::new(8);
        ring.insert_at_first(0.0);
        let _ = s_history_graph(8, &ring, 0.0, 0.0, 3, Color::Black);
    }

    #[test]
    fn history_graph_puts_newest_sample_on_the_right() {
        let mut ring = Ring::new(8);
        for v in [10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0] {
            ring.insert_at_first(v);
        }
        ring.insert_at_first(100.0);

        let spans = s_history_graph(4, &ring, 100.0, 0.0, 1, Color::Black);
        let last = spans[0].content.chars().last();
        // Newest pair is (100, 80) → braille '⣾'. The broken forward walk
        // paired (100, 20) and put '⣸' on the right edge instead.
        assert_eq!(last, Some('⣾'));
    }
}
