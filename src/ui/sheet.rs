//! The one table shape every command renders: named columns, aligned cells, optional total row.

use comfy_table::{Attribute, Cell, CellAlignment, ColumnConstraint};
use std::fmt::{self, Display};

use super::layout::{keep_tail, shorten_path, width};
use super::{Theme, table_at, table_width};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
}

pub struct Sheet {
    theme: Theme,
    headers: Vec<String>,
    aligns: Vec<Align>,
    rows: Vec<Vec<Cell>>,
    total: Option<Vec<Cell>>,
    flex: Option<usize>,
    headless: bool,
    optional: Vec<usize>,
}

impl Sheet {
    pub fn new(theme: Theme, columns: &[(&str, Align)]) -> Self {
        Self {
            theme,
            headers: columns.iter().map(|(name, _)| name.to_string()).collect(),
            aligns: columns.iter().map(|(_, align)| *align).collect(),
            rows: Vec::new(),
            total: None,
            flex: None,
            headless: false,
            optional: Vec::new(),
        }
    }

    /// The column that takes the remaining width. Its long single-word cells (paths, hashes,
    /// image tags) are shortened from the left with an ellipsis instead of being split.
    pub fn flex(mut self, index: usize) -> Self {
        self.flex = Some(index);
        self
    }

    /// Render without the header row (key/value panels).
    pub fn headless(mut self) -> Self {
        self.headless = true;
        self
    }

    /// Hide this column when every cell in it is empty or `-`.
    pub fn optional(mut self, index: usize) -> Self {
        self.optional.push(index);
        self
    }

    pub fn row(&mut self, cells: Vec<Cell>) {
        assert_eq!(cells.len(), self.headers.len(), "{:?}", self.headers);
        self.rows.push(cells);
    }

    pub fn total(&mut self, cells: Vec<Cell>) {
        assert_eq!(cells.len(), self.headers.len(), "{:?}", self.headers);
        let theme = self.theme;
        self.total = Some(
            cells
                .into_iter()
                .map(|cell| {
                    if theme.color() {
                        cell.add_attribute(Attribute::Bold)
                    } else {
                        cell
                    }
                })
                .collect(),
        );
    }

    pub fn headers(&self) -> &[String] {
        &self.headers
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn column(&self, index: usize) -> Vec<String> {
        self.rows
            .iter()
            .chain(self.total.iter())
            .map(|row| row[index].content())
            .collect()
    }

    pub fn render(&self) -> String {
        self.render_at(table_width())
    }

    fn hidden(&self, index: usize) -> bool {
        self.optional.contains(&index)
            && self
                .rows
                .iter()
                .chain(self.total.iter())
                .all(|row| matches!(row[index].content().trim(), "" | "-"))
    }

    /// Renders for a terminal `total` columns wide, or unbounded when `None` (pipes).
    pub fn render_at(&self, total: Option<usize>) -> String {
        let keep: Vec<usize> = (0..self.headers.len())
            .filter(|index| !self.hidden(*index))
            .collect();
        let headers: Vec<&str> = if self.headless {
            Vec::new()
        } else {
            keep.iter()
                .map(|index| self.headers[*index].as_str())
                .collect()
        };
        let room = self.flex_room_at(total, &keep);
        let mut table = table_at(self.theme, &headers, total);
        for row in self.rows.iter().chain(self.total.iter()) {
            table.add_row(keep.iter().map(|index| {
                let cell = &row[*index];
                match room {
                    Some(room) if Some(*index) == self.flex => self.fitted(cell, room),
                    _ => cell.clone(),
                }
            }));
        }
        for (position, column) in table.column_iter_mut().enumerate() {
            let index = keep[position];
            if Some(index) != self.flex {
                column.set_constraint(ColumnConstraint::ContentWidth);
            }
            column.set_cell_alignment(match self.aligns[index] {
                Align::Left => CellAlignment::Left,
                Align::Right => CellAlignment::Right,
            });
        }
        table.to_string()
    }

    fn flex_room_at(&self, total: Option<usize>, keep: &[usize]) -> Option<usize> {
        let flex = self.flex.filter(|flex| keep.contains(flex))?;
        let others: Vec<usize> = keep
            .iter()
            .filter(|index| **index != flex)
            .map(|index| {
                let header = if self.headless {
                    ""
                } else {
                    self.headers[*index].as_str()
                };
                let cells = self.column(*index);
                column_width(header, cells.iter().map(String::as_str))
            })
            .collect();
        flex_room(total?, &others)
    }

    fn fitted(&self, cell: &Cell, room: usize) -> Cell {
        let text = cell.content();
        if width(&text) <= room || text.contains(char::is_whitespace) {
            return cell.clone();
        }
        let ellipsis = self.theme.icons().ellipsis;
        let short = match super::classify(&text) {
            super::Token::Path => shorten_path(&text, room, ellipsis),
            _ => keep_tail(&text, room, ellipsis),
        };
        self.theme.token_cell(&short)
    }
}

impl Display for Sheet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render())
    }
}

pub const MIN_FLEX: usize = 16;

/// Content width left for the flexible column once the others, their padding, and the
/// borders are placed in a table `total` columns wide.
pub fn flex_room(total: usize, others: &[usize]) -> Option<usize> {
    let columns = others.len() + 1;
    let fixed = others.iter().map(|width| width + 2).sum::<usize>() + columns + 1;
    let room = total.checked_sub(fixed + 2)?;
    (room >= MIN_FLEX).then_some(room)
}

pub fn column_width<'a>(header: &str, cells: impl IntoIterator<Item = &'a str>) -> usize {
    cells
        .into_iter()
        .map(super::layout::width)
        .chain([super::layout::width(header)])
        .max()
        .unwrap_or(0)
}

pub fn is_bare_number(text: &str) -> bool {
    let text = text.trim();
    !text.is_empty()
        && text
            .chars()
            .all(|ch| ch.is_ascii_digit() || matches!(ch, ',' | '.'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::layout::{strip_ansi, width};

    fn sample(theme: Theme) -> Sheet {
        let mut sheet = Sheet::new(theme, &[("kind", Align::Left), ("services", Align::Right)]);
        sheet.row(vec![theme.cell("php", None, &[]), Cell::new("52")]);
        sheet.row(vec![theme.cell("vue", None, &[]), Cell::new("4")]);
        sheet.total(vec![Cell::new("all kinds"), Cell::new("56")]);
        sheet
    }

    #[test]
    fn renders_a_header_and_aligned_rows() {
        let plain = sample(Theme::plain()).render();
        let lines: Vec<&str> = plain.lines().collect();
        assert!(lines[1].contains("kind") && lines[1].contains("services"));
        assert!(lines.iter().all(|line| width(line) == width(lines[0])));
        assert!(lines[3].ends_with("52 │"));
        assert!(lines[4].ends_with(" 4 │"));
        assert!(plain.contains("all kinds"));
        assert_eq!(strip_ansi(&sample(Theme::colored()).render()), plain);
    }

    #[test]
    fn exposes_columns_for_checks() {
        let sheet = sample(Theme::plain());
        assert_eq!(sheet.headers(), ["kind", "services"]);
        assert_eq!(sheet.column(1), ["52", "4", "56"]);
        assert_eq!(sheet.len(), 2);
        assert!(is_bare_number("1,092"));
        assert!(!is_bare_number("72.2%"));
        assert!(!is_bare_number("4 chats"));
    }

    #[test]
    fn flex_room_leaves_space_for_the_other_columns() {
        assert_eq!(flex_room(40, &[4, 6]), Some(40 - (6 + 8 + 4) - 2));
        assert_eq!(flex_room(20, &[4, 6]), None);
        assert_eq!(column_width("kind", ["● php.Dockerfile", "x"]), 16);
        assert_eq!(column_width("services", ["52"]), 8);
    }

    fn assert_fits(text: &str, total: usize) {
        let lines: Vec<&str> = text.lines().collect();
        for line in &lines {
            assert!(width(line) <= total, "{} > {total}: {line:?}", width(line));
            assert_eq!(width(line), width(lines[0]), "ragged frame: {line:?}");
        }
    }

    #[test]
    fn panels_have_no_header_and_shorten_long_paths() {
        let theme = Theme::plain();
        let path = "~/OrbStack/resolute/home/someone/projects/resolved/api/with/a/very/long/tail";
        let panel = crate::ui::panel(
            theme,
            vec![
                ("root", theme.token_cell(path)),
                ("build order", Cell::new("bundler, php")),
            ],
        );
        for total in [60, 80, 120] {
            let text = panel.render_at(Some(total));
            assert_fits(&text, total);
            assert!(!text.contains("field"), "no header row: {text}");
            assert_eq!(text.lines().count(), 4, "{text}");
            assert!(text.contains("long/tail"), "keeps the tail: {text}");
        }
        assert!(panel.render_at(Some(60)).contains('…'));
        assert!(
            panel.render_at(None).contains(path),
            "pipes get the full value"
        );
    }

    #[test]
    fn optional_columns_disappear_when_empty() {
        let theme = Theme::plain();
        let mut sheet = Sheet::new(
            theme,
            &[
                ("step", Align::Left),
                ("status", Align::Left),
                ("detail", Align::Left),
            ],
        )
        .flex(2)
        .optional(2);
        sheet.row(vec![
            Cell::new("Identity"),
            Cell::new("not set"),
            Cell::new("-"),
        ]);
        sheet.row(vec![
            Cell::new("Docker"),
            Cell::new("not set"),
            Cell::new(""),
        ]);
        let text = sheet.render_at(Some(80));
        assert!(!text.contains("detail"), "{text}");
        assert_fits(&text, 80);
        sheet.row(vec![
            Cell::new("GitHub"),
            Cell::new("done"),
            Cell::new("via gh"),
        ]);
        assert!(sheet.render_at(Some(80)).contains("detail"));
    }

    #[test]
    fn wide_tables_fit_narrow_terminals() {
        let theme = Theme::plain();
        let mut sheet = Sheet::new(
            theme,
            &[
                ("key", Align::Left),
                ("scope", Align::Left),
                ("value", Align::Left),
            ],
        )
        .flex(2);
        sheet.row(vec![
            Cell::new("openrouter.model"),
            Cell::new("global"),
            Cell::new("a long sentence value that has to wrap over several lines in the cell"),
        ]);
        sheet.row(vec![
            Cell::new("sha256"),
            Cell::new("project"),
            theme.token_cell(&"f".repeat(64)),
        ]);
        for total in [40, 60, 80] {
            assert_fits(&sheet.render_at(Some(total)), total);
        }
    }

    #[test]
    #[should_panic]
    fn rows_must_match_the_header() {
        let mut sheet = Sheet::new(Theme::plain(), &[("a", Align::Left)]);
        sheet.row(vec![Cell::new("1"), Cell::new("2")]);
    }
}
