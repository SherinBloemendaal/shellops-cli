mod progress;
mod prompt;
pub mod term;

pub use progress::{Spinner, spinner};
pub use prompt::{confirm, confirm_default, multi_select, select};

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
    pub sep: &'static str,
    pub header: &'static str,
}

pub const UNICODE: Icons = Icons {
    check: "✔",
    cross: "✖",
    warn: "⚠",
    info: "ℹ",
    arrow: "➜",
    sep: " · ",
    header: "==>",
};

pub const ASCII: Icons = Icons {
    check: "+",
    cross: "x",
    warn: "!",
    info: "i",
    arrow: "->",
    sep: " | ",
    header: "==>",
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    color: bool,
    unicode: bool,
}

impl Theme {
    pub const fn plain() -> Self {
        Self {
            color: false,
            unicode: true,
        }
    }

    pub const fn colored() -> Self {
        Self {
            color: true,
            unicode: true,
        }
    }

    pub fn stdout() -> Self {
        let settings = term::settings();
        Self {
            color: settings.stdout,
            unicode: settings.unicode,
        }
    }

    pub fn stderr() -> Self {
        let settings = term::settings();
        Self {
            color: settings.stderr,
            unicode: settings.unicode,
        }
    }

    pub fn color(self) -> bool {
        self.color
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

    pub fn heading(self, text: impl Display) -> String {
        self.paint(text, Style::new().bold().blue())
    }

    pub fn path(self, text: impl Display) -> String {
        self.paint(text, Style::new().cyan())
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

    pub fn arrow(self) -> String {
        self.dim(self.icons().arrow)
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

pub fn section(title: &str) {
    println!("{}", section_line(Theme::stdout(), title));
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

pub fn error_text(theme: Theme, err: &anyhow::Error) -> String {
    let mut chain = err.chain();
    let headline = chain
        .next()
        .map(|head| match head.downcast_ref::<Hinted>() {
            Some(hinted) => hinted.message.clone(),
            None => head.to_string(),
        })
        .unwrap_or_default();
    let mut out = err_line(theme, &headline);
    for cause in chain {
        let text = match cause.downcast_ref::<Hinted>() {
            Some(hinted) => hinted.message.clone(),
            None => cause.to_string(),
        };
        out.push('\n');
        out.push_str(&format!("  {} {text}", theme.icons().arrow));
    }
    if let Some(hinted) = err.chain().find_map(|cause| cause.downcast_ref::<Hinted>()) {
        out.push('\n');
        out.push_str(&format!("  {}", hinted.hint));
    }
    out
}

pub fn report_error(err: &anyhow::Error) {
    eprintln!("{}", error_text(Theme::stderr(), err));
}

pub fn table(theme: Theme, headers: &[&str]) -> Table {
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
    if theme.color {
        table.enforce_styling();
    }
    if !headers.is_empty() {
        table.set_header(
            headers
                .iter()
                .map(|header| theme.cell(header, Some(Color::Cyan), &[Attribute::Bold])),
        );
    }
    table
}

pub fn panel(theme: Theme, rows: Vec<(&str, String)>) -> String {
    let mut table = table(theme, &[]);
    for (key, value) in rows {
        table.add_row(vec![
            theme.cell(key, Some(Color::Blue), &[Attribute::Bold]),
            Cell::new(value).set_alignment(CellAlignment::Left),
        ]);
    }
    table.to_string()
}

pub fn stdout_is_tty() -> bool {
    io::stdout().is_terminal()
}

pub fn stdin_is_tty() -> bool {
    io::stdin().is_terminal()
}

pub fn print_help() {
    let theme = Theme::stdout();
    println!("{}", section_line(theme, "shellops"));
    println!(
        "  {}",
        theme.dim("Docker, PHP, and release commands for a compose project.")
    );
    println!();
    let rows = [
        (
            "compose",
            "Run docker compose, injecting UID, GID, and USER",
        ),
        ("build", "Build *.Dockerfile images in dependency order"),
        ("cc", "Clear caches in three parallel phases"),
        ("console, c", "Symfony console"),
        ("composer, cp", "Composer, with install flags kept"),
        ("dump", "composer dump-autoload"),
        ("csf, csfixer", "PHP CS Fixer"),
        ("phpstan", "PHPStan"),
        ("phan", "Phan, with a file list when paths are passed"),
        ("phpunit", "PHPUnit"),
        ("rector", "Rector"),
        ("phpmd", "PHPMD"),
        ("fixtures, f", "Load Doctrine fixtures"),
        ("reset-database, rd", "Drop, migrate, and load fixtures"),
        (
            "force-drop-database, fdd",
            "dropdb -f the Postgres database",
        ),
        ("installer", "Rebuild, install, and load a fresh database"),
        ("yarn", "Yarn inside the vue service"),
        ("debug", "Show the detected project"),
        ("sort-dotenv", "Sort a dotenv file in place"),
        ("keypair", "Write an OpenSSL key pair"),
        ("randomstr", "Print a random string"),
        ("config", "User and project settings, including secrets"),
        ("release", "Bump, tag, and optionally publish notes"),
        ("update", "Install the latest so release"),
        ("github", "Open the shellops-cli repository"),
    ];
    let mut sheet = table(theme, &["command", "what it does"]);
    for (command, about) in rows {
        sheet.add_row(vec![
            theme.cell(command, Some(Color::Cyan), &[Attribute::Bold]),
            Cell::new(about),
        ]);
    }
    println!("{sheet}");
    println!();
    println!(
        "  {} {}",
        theme.dim("Config lives in"),
        theme.path("~/.shellops")
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_lines_have_no_escapes() {
        let theme = Theme::plain();
        assert_eq!(section_line(theme, "Title"), "==> Title");
        assert_eq!(ok_line(theme, "done"), "✔ done");
        assert!(!Theme::colored().heading("x").is_empty());
        assert!(Theme::colored().heading("x").contains('\u{1b}'));
    }
}
