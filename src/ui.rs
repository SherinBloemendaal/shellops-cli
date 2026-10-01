//! Terminal presentation: themed tables, prompts, and progress.

mod banner;
mod help;
pub mod layout;
mod progress;
mod prompt;
mod sheet;
pub mod signal;
pub mod term;

pub use help::{help_text, print_help};
pub use layout::{Grid, tilde};
pub use progress::{
    Bars, DualProgress, Spinner, Transfer, done_line, elapsed_text, spinner, transfer,
};
pub use prompt::{confirm, confirm_default, fuzzy, input, multi_select, password, select};
pub use sheet::{Align, Sheet, column_width, flex_room, is_bare_number};
pub use term::{ColorMode, Depth};

use comfy_table::modifiers::UTF8_ROUND_CORNERS;
use comfy_table::presets::{ASCII_FULL_CONDENSED, UTF8_FULL_CONDENSED};
use comfy_table::{
    Attribute, Cell, CellAlignment, Color, ContentArrangement, Table, TableComponent,
};
use owo_colors::{OwoColorize, Style};
use std::fmt::{self, Display};
use std::io::{self, IsTerminal};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Icons {
    pub check: &'static str,
    pub cross: &'static str,
    pub warn: &'static str,
    pub info: &'static str,
    pub arrow: &'static str,
    pub bullet: &'static str,
    pub diamond: &'static str,
    pub pointer: &'static str,
    pub hollow: &'static str,
    pub square: &'static str,
    pub sep: &'static str,
    pub ellipsis: &'static str,
    pub header: &'static str,
}

pub const UNICODE: Icons = Icons {
    check: "✔",
    cross: "✖",
    warn: "⚠",
    info: "ℹ",
    arrow: "➜",
    bullet: "●",
    diamond: "◆",
    pointer: "▸",
    hollow: "○",
    square: "■",
    sep: " · ",
    ellipsis: "…",
    header: "==>",
};

pub const ASCII: Icons = Icons {
    check: "+",
    cross: "x",
    warn: "!",
    info: "i",
    arrow: "->",
    bullet: "*",
    diamond: "*",
    pointer: ">",
    hollow: "-",
    square: "#",
    sep: " | ",
    ellipsis: "...",
    header: "==>",
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    color: bool,
    depth: Depth,
    unicode: bool,
}

impl Theme {
    pub const fn plain() -> Self {
        Self {
            color: false,
            depth: Depth::Basic,
            unicode: true,
        }
    }

    pub const fn colored() -> Self {
        Self {
            color: true,
            depth: Depth::Basic,
            unicode: true,
        }
    }

    pub const fn ascii() -> Self {
        Self {
            color: false,
            depth: Depth::Basic,
            unicode: false,
        }
    }

    pub const fn with_depth(self, depth: Depth) -> Self {
        Self { depth, ..self }
    }

    pub const fn without_color(self) -> Self {
        Self {
            color: false,
            ..self
        }
    }

    pub fn stdout() -> Self {
        let settings = term::settings();
        Self {
            color: settings.stdout,
            depth: settings.depth,
            unicode: settings.unicode,
        }
    }

    pub fn stderr() -> Self {
        let settings = term::settings();
        Self {
            color: settings.stderr,
            depth: settings.depth,
            unicode: settings.unicode,
        }
    }

    pub fn color(self) -> bool {
        self.color
    }

    pub fn depth(self) -> Depth {
        self.depth
    }

    pub fn unicode(self) -> bool {
        self.unicode
    }

    pub fn icons(self) -> &'static Icons {
        if self.unicode { &UNICODE } else { &ASCII }
    }

    pub fn sep(self) -> String {
        self.dim(self.icons().sep)
    }

    pub fn paint(self, text: impl Display, style: Style) -> String {
        if self.color {
            text.style(style).to_string()
        } else {
            text.to_string()
        }
    }

    pub fn bold(self, text: impl Display) -> String {
        self.paint(text, Style::new().bold())
    }

    pub fn dim(self, text: impl Display) -> String {
        self.paint(text, Style::new().dimmed())
    }

    /// The ShellOps brand accent: phosphor green, matching the banner.
    pub fn accent(self, text: impl Display) -> String {
        self.paint(text, Style::new().bold().green())
    }

    pub fn heading(self, text: impl Display) -> String {
        self.accent(text)
    }

    pub fn path(self, text: impl Display) -> String {
        self.paint(text, Style::new().cyan())
    }

    pub fn id(self, text: impl Display) -> String {
        self.paint(text, Style::new().magenta())
    }

    pub fn good(self, text: impl Display) -> String {
        self.paint(text, Style::new().green())
    }

    pub fn bad(self, text: impl Display) -> String {
        self.paint(text, Style::new().red())
    }

    pub fn caution(self, text: impl Display) -> String {
        self.paint(text, Style::new().yellow())
    }

    pub fn command(self, text: impl Display) -> String {
        self.paint(text, Style::new().bold().cyan())
    }

    pub fn flag(self, text: impl Display) -> String {
        self.paint(text, Style::new().yellow())
    }

    pub fn number(self, text: impl Display) -> String {
        self.paint(text, Style::new().bold())
    }

    pub fn arrow(self) -> String {
        self.dim(self.icons().arrow)
    }

    pub fn token(self, text: &str) -> String {
        match classify(text) {
            Token::Path => self.path(text),
            Token::Id => self.id(text),
            Token::Plain => text.to_string(),
        }
    }

    pub fn cell(self, text: impl Display, color: Option<Color>, attributes: &[Attribute]) -> Cell {
        let mut cell = Cell::new(text);
        if self.color {
            if let Some(color) = color {
                cell = cell.fg(color);
            }
            for attribute in attributes {
                cell = cell.add_attribute(*attribute);
            }
        }
        cell
    }

    pub fn token_cell(self, text: &str) -> Cell {
        match classify(text) {
            Token::Path => self.cell(text, Some(Color::Cyan), &[]),
            Token::Id => self.cell(text, Some(Color::Magenta), &[]),
            Token::Plain => Cell::new(text),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    Path,
    Id,
    Plain,
}

pub fn classify(text: &str) -> Token {
    let is_path = text.starts_with('/')
        || text.starts_with("~/")
        || text.starts_with("./")
        || text.contains('/')
        || text.contains('\\');
    if is_path {
        return Token::Path;
    }
    let hexish = text.len() >= 8
        && text.chars().any(|ch| ch.is_ascii_digit())
        && text.chars().all(|ch| ch.is_ascii_hexdigit() || ch == '-');
    if hexish || (!text.is_empty() && text.chars().all(|ch| ch.is_ascii_digit())) {
        return Token::Id;
    }
    Token::Plain
}

pub fn section_line(theme: Theme, title: &str) -> String {
    format!(
        "{} {}",
        theme.heading(theme.icons().header),
        theme.bold(title)
    )
}

pub fn ok_line(theme: Theme, message: &str) -> String {
    format!("{} {message}", theme.good(theme.icons().check))
}

pub fn warn_line(theme: Theme, message: &str) -> String {
    format!("{} {message}", theme.caution(theme.icons().warn))
}

pub fn err_line(theme: Theme, message: &str) -> String {
    format!(
        "{} {}",
        theme.paint(theme.icons().cross, Style::new().bold().red()),
        theme.paint(message, Style::new().bold().red())
    )
}

pub fn info_line(theme: Theme, message: &str) -> String {
    format!(
        "{} {message}",
        theme.paint(theme.icons().info, Style::new().blue())
    )
}

pub fn hint_line(theme: Theme, message: &str) -> String {
    format!("  {}", theme.dim(message))
}

/// `▸ message  2/5`: one step of a multi-step command whose tools print their own output.
pub fn step_line(theme: Theme, index: usize, total: usize, message: &str) -> String {
    let counter = if total > 1 {
        format!("  {}", theme.dim(format!("{index}/{total}")))
    } else {
        String::new()
    };
    format!(
        "{} {}{counter}",
        theme.accent(theme.icons().pointer),
        theme.bold(message)
    )
}

/// `  • item`: one entry of a short list under a warning or an info line.
pub fn item_line(theme: Theme, item: &str) -> String {
    format!("  {} {item}", theme.dim(theme.icons().bullet))
}

/// Wraps `text` to `total` columns behind `first` (whose visible width is `indent`), hanging
/// continuation lines at `indent`. Line breaks in `text` are kept.
fn hang(
    first: &str,
    indent: usize,
    text: &str,
    total: usize,
    paint: impl Fn(&str) -> String,
) -> String {
    let room = total.saturating_sub(indent).max(16);
    let mut out = String::new();
    for (index, line) in text
        .lines()
        .flat_map(|line| layout::wrap(line, room))
        .enumerate()
    {
        if index == 0 {
            out.push_str(first);
        } else {
            out.push('\n');
            out.push_str(&" ".repeat(indent));
        }
        out.push_str(&paint(&line));
    }
    if out.is_empty() {
        out.push_str(first.trim_end());
    }
    out
}

pub fn section(title: &str) {
    println!("{}", section_line(Theme::stdout(), title));
}

/// Prints `▸ message  2/5` and starts timing the step; end it with [`Step::done`] or
/// [`Step::fail`] so the time it took is printed.
pub fn step(index: usize, total: usize, message: &str) -> Step {
    println!("{}", step_line(Theme::stdout(), index, total, message));
    Step {
        message: message.to_string(),
        started: std::time::Instant::now(),
    }
}

#[must_use = "end the step with done or fail so its time is printed"]
pub struct Step {
    message: String,
    started: std::time::Instant,
}

impl Step {
    /// `✔ message  3.1s`
    pub fn done(self) {
        let message = self.message.clone();
        self.done_as(&message);
    }

    /// `✔ message  3.1s`, with a message that says what the step produced.
    pub fn done_as(self, message: &str) {
        println!(
            "{}",
            done_line(Theme::stdout(), true, message, self.started.elapsed())
        );
    }

    /// `✖ message failed (exit 2)  3.1s`, returning `code` for the caller to exit with.
    pub fn fail(self, code: i32) -> i32 {
        eprintln!(
            "{}",
            done_line(
                Theme::stderr(),
                false,
                &format!("{} failed (exit {code})", self.message),
                self.started.elapsed()
            )
        );
        code
    }
}

pub fn item(text: &str) {
    println!("{}", item_line(Theme::stdout(), text));
}

pub fn ok(message: &str) {
    println!("{}", ok_line(Theme::stdout(), message));
}

pub fn warn(message: &str) {
    eprintln!("{}", warn_line(Theme::stderr(), message));
}

pub fn notice(message: &str) {
    println!("{}", warn_line(Theme::stdout(), message));
}

pub fn info(message: &str) {
    println!("{}", info_line(Theme::stdout(), message));
}

pub fn hint(message: &str) {
    println!("{}", hint_line(Theme::stdout(), message));
}

pub fn err(message: &str) {
    eprintln!("{}", err_line(Theme::stderr(), message));
}

#[derive(Debug)]
pub struct Hinted {
    pub message: String,
    pub hint: String,
}

impl Display for Hinted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let joiner = if self.message.ends_with(['.', '?', '!']) {
            " "
        } else {
            ". "
        };
        write!(f, "{}{joiner}{}", self.message, self.hint)
    }
}

impl std::error::Error for Hinted {}

pub fn hinted(message: impl Into<String>, hint: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(Hinted {
        message: message.into(),
        hint: hint.into(),
    })
}

/// The error block: a bold headline, one `➜` line per cause, then the hint. Every line is
/// wrapped to `total` columns with a hanging indent, so nothing runs past the terminal edge.
pub fn error_text(theme: Theme, err: &anyhow::Error, total: usize) -> String {
    let mut chain = err.chain();
    let headline = chain
        .next()
        .map(|head| match head.downcast_ref::<Hinted>() {
            Some(hinted) => hinted.message.clone(),
            None => head.to_string(),
        })
        .unwrap_or_default();
    let causes: Vec<String> = chain
        .map(|cause| match cause.downcast_ref::<Hinted>() {
            Some(hinted) => hinted.message.clone(),
            None => cause.to_string(),
        })
        .collect();
    let hint = err
        .chain()
        .find_map(|cause| cause.downcast_ref::<Hinted>())
        .map(|hinted| hinted.hint.clone());
    failure_text(theme, &headline, &causes, hint.as_deref(), total)
}

pub fn failure_text(
    theme: Theme,
    headline: &str,
    causes: &[String],
    hint: Option<&str>,
    total: usize,
) -> String {
    let icons = theme.icons();
    let bad = Style::new().bold().red();
    let mut out = hang(
        &format!("{} ", theme.paint(icons.cross, bad)),
        2,
        headline,
        total,
        |line| theme.paint(line, bad),
    );
    let arrow = format!("  {} ", theme.dim(icons.arrow));
    let arrow_width = 2 + layout::width(icons.arrow) + 1;
    for cause in causes {
        out.push('\n');
        out.push_str(&hang(&arrow, arrow_width, cause, total, |line| {
            theme.dim(line)
        }));
    }
    if let Some(hint) = hint {
        out.push('\n');
        out.push_str(&hang("  ", 2, hint, total, |line| theme.dim(line)));
    }
    out
}

pub fn report_error(err: &anyhow::Error) {
    eprintln!("{}", error_text(Theme::stderr(), err, term::stderr_width()));
}

/// Restyles a clap parse error (`error: …`, then usage and tips) as a ShellOps error block.
pub fn clap_error_text(theme: Theme, rendered: &str, total: usize) -> String {
    let plain = layout::strip_ansi(rendered);
    let mut lines = plain.lines().map(str::trim_end);
    let headline = lines
        .next()
        .map(|line| line.strip_prefix("error: ").unwrap_or(line).to_string())
        .unwrap_or_default();
    let mut causes = Vec::new();
    let mut hint = None;
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with("Usage:") {
            hint = Some(line.to_string());
        } else if line.starts_with("For more information") {
            continue;
        } else {
            causes.push(line.to_string());
        }
    }
    failure_text(theme, &headline, &causes, hint.as_deref(), total)
}

/// Wraps a clap help screen to `total` columns. clap is built without `wrap_help`, so long
/// argument descriptions are wrapped here, hanging under the description column.
pub fn wrap_help(text: &str, total: usize) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let (body, newline) = match line.strip_suffix('\n') {
            Some(body) => (body, "\n"),
            None => (line, ""),
        };
        out.push_str(&wrap_help_line(body, total));
        out.push_str(newline);
    }
    out
}

fn wrap_help_line(line: &str, total: usize) -> String {
    if layout::width(line) <= total {
        return line.to_string();
    }
    let plain = layout::strip_ansi(line);
    let indent = plain.len() - plain.trim_start().len();
    let Some(gap) = plain[indent..].find("  ").map(|at| at + indent) else {
        return hang("", indent, line, total, str::to_string);
    };
    let start = gap + plain[gap..].len() - plain[gap..].trim_start().len();
    let description = &plain[start..];
    let Some(spec) = line.strip_suffix(description) else {
        return line.to_string();
    };
    let column = layout::width(&plain[..start]);
    if column + 16 > total {
        return line.to_string();
    }
    hang(spec, column, description, total, str::to_string)
}

pub fn table(theme: Theme, headers: &[&str]) -> Table {
    table_at(theme, headers, table_width())
}

pub fn table_at(theme: Theme, headers: &[&str], width: Option<usize>) -> Table {
    let mut table = Table::new();
    if theme.unicode {
        table
            .load_preset(UTF8_FULL_CONDENSED)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_style(TableComponent::VerticalLines, '│')
            .set_style(TableComponent::HeaderLines, '─')
            .set_style(TableComponent::LeftHeaderIntersection, '├')
            .set_style(TableComponent::MiddleHeaderIntersections, '┼')
            .set_style(TableComponent::RightHeaderIntersection, '┤');
    } else {
        table.load_preset(ASCII_FULL_CONDENSED);
    }
    table.set_content_arrangement(ContentArrangement::Dynamic);
    if let Some(total) = width {
        table.set_width(u16::try_from(total).unwrap_or(u16::MAX));
    }
    if theme.color {
        table.enforce_styling();
    }
    if !headers.is_empty() {
        table.set_header(
            headers
                .iter()
                .map(|header| theme.cell(header, Some(ACCENT), &[Attribute::Bold])),
        );
    }
    table
}

/// The brand accent as a table color.
pub const ACCENT: Color = Color::Green;

/// A key/value panel: no header row, bold keys, values in a flexible column. Long single-word
/// values (paths, hashes) are shortened with an ellipsis instead of being split mid-word.
pub fn panel(theme: Theme, rows: Vec<(&str, Cell)>) -> Sheet {
    let mut sheet = Sheet::new(theme, &[("field", Align::Left), ("value", Align::Left)])
        .flex(1)
        .headless();
    for (key, value) in rows {
        sheet.row(vec![
            theme.cell(key, Some(ACCENT), &[Attribute::Bold]),
            value.set_alignment(CellAlignment::Left),
        ]);
    }
    sheet
}

pub fn stdout_is_tty() -> bool {
    io::stdout().is_terminal()
}

pub fn stdin_is_tty() -> bool {
    io::stdin().is_terminal()
}

pub fn table_width() -> Option<usize> {
    stdout_is_tty().then(term::width)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_theme_emits_no_escape_codes() {
        let theme = Theme::plain();
        assert_eq!(theme.heading("x"), "x");
        assert_eq!(section_line(theme, "Title"), "==> Title");
        assert_eq!(ok_line(theme, "done"), "✔ done");
        assert_eq!(warn_line(theme, "careful"), "⚠ careful");
        assert_eq!(err_line(theme, "broke"), "✖ broke");
        assert_eq!(info_line(theme, "note"), "ℹ note");
        assert!(Theme::colored().heading("x").contains('\u{1b}'));
    }

    #[test]
    fn long_errors_wrap_with_a_hanging_indent() {
        let err = hinted(
            "not inside a git repository: fatal: not a git repository (or any of the parent directories): .git",
            "Run so from inside a project checkout, or pass the path of one with cd first.",
        )
        .context("could not find the project root for this command");
        for total in [40, 60, 80] {
            for theme in [Theme::plain(), Theme::colored(), Theme::ascii()] {
                let text = error_text(theme, &err, total);
                for line in text.lines() {
                    assert!(
                        layout::width(line) <= total,
                        "{} > {total}: {line:?}",
                        layout::width(line)
                    );
                }
                assert_eq!(
                    layout::strip_ansi(&text),
                    error_text(theme.without_color(), &err, total)
                );
            }
        }
        let text = error_text(Theme::plain(), &err, 40);
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].starts_with("✖ could not find"));
        assert!(lines[1].starts_with("  "), "{lines:?}");
        assert!(lines.iter().any(|line| line.starts_with("  ➜ not inside")));
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("    ") && !line.trim().is_empty()),
            "cause continuation hangs under the cause text: {lines:?}"
        );
    }

    #[test]
    fn clap_errors_use_the_shellops_error_block() {
        let rendered = "error: unexpected argument 'x' found\n\nUsage: so init [OPTIONS] [STEP]\n\nFor more information, try '--help'.\n";
        let text = clap_error_text(Theme::plain(), rendered, 80);
        assert_eq!(
            text,
            "✖ unexpected argument 'x' found\n  Usage: so init [OPTIONS] [STEP]"
        );
        let missing = "error: the following required arguments were not provided:\n  <LENGTH>\n\nUsage: so randomstr <LENGTH>\n";
        assert_eq!(
            clap_error_text(Theme::ascii(), missing, 80),
            "x the following required arguments were not provided:\n  -> <LENGTH>\n  Usage: so randomstr <LENGTH>"
        );
    }

    #[test]
    fn clap_help_wraps_under_the_description_column() {
        let help = "Set a key\n\nOptions:\n      --secret        Store as a secret: in secrets.toml, masked in lists and never printed by get\n  -h, --help          Print help\n";
        let wrapped = wrap_help(help, 60);
        for line in wrapped.lines() {
            assert!(layout::width(line) <= 60, "{line:?}");
        }
        let lines: Vec<&str> = wrapped.lines().collect();
        assert!(lines[3].starts_with("      --secret        Store as a secret:"));
        assert!(lines[4].starts_with(&" ".repeat(22)), "{lines:?}");
        assert!(!lines[4].starts_with(&" ".repeat(23)), "{lines:?}");
        assert_eq!(lines[lines.len() - 1], "  -h, --help          Print help");
        assert!(wrapped.ends_with('\n'));
        let colored = "      \u{1b}[1m--secret\u{1b}[0m        Store as a secret: in secrets.toml, masked in lists and never printed by get";
        let wrapped = wrap_help(colored, 60);
        assert!(
            wrapped.lines().all(|line| layout::width(line) <= 60),
            "{wrapped:?}"
        );
        assert_eq!(
            layout::strip_ansi(&wrapped),
            wrap_help(&layout::strip_ansi(colored), 60)
        );
    }

    #[test]
    fn step_and_item_lines() {
        assert_eq!(
            step_line(Theme::plain(), 2, 5, "Building php"),
            "▸ Building php  2/5"
        );
        assert_eq!(step_line(Theme::ascii(), 1, 1, "Building"), "> Building");
        assert_eq!(item_line(Theme::plain(), "a/b:latest"), "  ● a/b:latest");
        assert!(Theme::colored().accent("x").contains("\u{1b}[32"));
    }

    #[test]
    fn ascii_theme_falls_back_to_plain_glyphs() {
        let theme = Theme::ascii();
        assert_eq!(ok_line(theme, "done"), "+ done");
        assert_eq!(warn_line(theme, "careful"), "! careful");
        assert_eq!(err_line(theme, "broke"), "x broke");
        let mut table = table(theme, &["a"]);
        table.add_row(vec!["b"]);
        assert!(table.to_string().is_ascii());
    }

    #[test]
    fn tokens_are_classified() {
        assert_eq!(classify("/Users/me/app"), Token::Path);
        assert_eq!(classify("src/Entity/Foo.php"), Token::Path);
        assert_eq!(classify("2e3fd84f5a21db3ee358105ab35ae586"), Token::Id);
        assert_eq!(classify("1764785046708"), Token::Id);
        assert_eq!(classify("remove"), Token::Plain);
        assert_eq!(classify("compose.yml"), Token::Plain);
    }

    #[test]
    fn hinted_errors_keep_their_text_and_split_the_hint() {
        let err = hinted("Docker is not running.", "Start Docker and retry.");
        assert_eq!(
            err.to_string(),
            "Docker is not running. Start Docker and retry."
        );
        let rendered = error_text(Theme::plain(), &err, 80);
        assert_eq!(
            rendered,
            "✖ Docker is not running.\n  Start Docker and retry."
        );
        let joined = hinted(
            "warnings need confirmation",
            "Answer the prompt to continue.",
        );
        assert_eq!(
            joined.to_string(),
            "warnings need confirmation. Answer the prompt to continue."
        );
        let wrapped = anyhow::anyhow!("disk full").context("failed to write");
        assert_eq!(
            error_text(Theme::plain(), &wrapped, 80),
            "✖ failed to write\n  ➜ disk full"
        );
    }
}
