//! The top-level `so` help screen: banner, then one strictly aligned grid.

use owo_colors::Style;
use std::io::{self, Write};

use super::banner::banner;
use super::layout::{Grid, wrap_indented};
use super::{Theme, section_line, term};

const INDENT: usize = 2;
/// Narrowest description column the wide layout accepts. Below it, arguments move next to the
/// command name so descriptions keep enough room instead of wrapping every few words.
const MIN_ABOUT: usize = 40;
/// Below this many description columns, examples put their description on the next line.
const MIN_EXAMPLE_ABOUT: usize = 24;

pub struct Entry {
    pub icon: (&'static str, &'static str),
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub args: &'static str,
    pub about: &'static str,
}

pub struct Group {
    pub title: &'static str,
    pub style: fn() -> Style,
    pub entries: &'static [Entry],
}

pub struct Flag {
    pub short: &'static str,
    pub long: &'static str,
    pub value: &'static str,
    pub about: &'static str,
}

const fn entry(
    icon: (&'static str, &'static str),
    name: &'static str,
    aliases: &'static [&'static str],
    args: &'static str,
    about: &'static str,
) -> Entry {
    Entry {
        icon,
        name,
        aliases,
        args,
        about,
    }
}

const fn flag(
    short: &'static str,
    long: &'static str,
    value: &'static str,
    about: &'static str,
) -> Flag {
    Flag {
        short,
        long,
        value,
        about,
    }
}

pub const GROUPS: &[Group] = &[
    Group {
        title: "Compose",
        style: || Style::new().cyan(),
        entries: &[
            entry(
                ("▸", ">"),
                "compose",
                &[],
                "[ARGS...]",
                "Run docker compose, injecting UID, GID, and USER",
            ),
            entry(
                ("⇧", "^"),
                "build",
                &[],
                "",
                "Build *.Dockerfile images in dependency order",
            ),
            entry(
                ("↻", "@"),
                "cc",
                &[],
                "",
                "Clear caches in three parallel phases",
            ),
        ],
    },
    Group {
        title: "PHP",
        style: || Style::new().magenta(),
        entries: &[
            entry(
                ("➜", ">"),
                "console",
                &["c"],
                "[ARGS...]",
                "Symfony console in the php service",
            ),
            entry(
                ("✚", "+"),
                "composer",
                &["cp"],
                "[ARGS...]",
                "Composer, with install flags kept",
            ),
            entry(("⇩", "v"), "dump", &[], "", "composer dump-autoload"),
            entry(("✎", "*"), "csf", &["csfixer"], "", "PHP CS Fixer"),
            entry(("●", "*"), "phpstan", &[], "[ARGS...]", "PHPStan"),
            entry(
                ("▤", "#"),
                "phan",
                &[],
                "[ARGS...]",
                "Phan, with a file list when paths are passed",
            ),
            entry(("✔", "+"), "phpunit", &[], "[ARGS...]", "PHPUnit"),
            entry(("↺", "~"), "rector", &[], "[ARGS...]", "Rector"),
            entry(("⚠", "!"), "phpmd", &[], "", "PHPMD"),
            entry(
                ("○", "-"),
                "yarn",
                &[],
                "[ARGS...]",
                "Yarn inside the vue service",
            ),
        ],
    },
    Group {
        title: "Data",
        style: || Style::new().yellow(),
        entries: &[
            entry(("◆", "*"), "fixtures", &["f"], "", "Load Doctrine fixtures"),
            entry(
                ("≡", "="),
                "reset-database",
                &["rd"],
                "",
                "Drop, migrate, and load fixtures",
            ),
            entry(
                ("✗", "x"),
                "force-drop-database",
                &["fdd"],
                "",
                "dropdb -f the Postgres database",
            ),
            entry(
                ("✦", "*"),
                "installer",
                &[],
                "",
                "Rebuild, install, and load a fresh database",
            ),
        ],
    },
    Group {
        title: "Project",
        style: || Style::new().green(),
        entries: &[
            entry(("◉", "*"), "debug", &[], "", "Show the detected project"),
            entry(
                ("≡", "="),
                "sort-dotenv",
                &[],
                "FILE",
                "Sort a dotenv file in place",
            ),
            entry(
                ("⊕", "+"),
                "keypair",
                &[],
                "DIR [SUFFIX]",
                "Write an OpenSSL key pair",
            ),
            entry(
                ("◆", "*"),
                "randomstr",
                &[],
                "LENGTH",
                "Print a random string",
            ),
            entry(
                ("◫", "%"),
                "config",
                &[],
                "set|get|list|unset",
                "User and project settings, including secrets",
            ),
            entry(
                ("◎", "*"),
                "init",
                &[],
                "[STEP]",
                "Set up identity and API keys",
            ),
            entry(
                ("✎", "*"),
                "alias",
                &[],
                "create|list|...",
                "Signed aliases, global or per repo",
            ),
        ],
    },
    Group {
        title: "Release",
        style: || Style::new().blue(),
        entries: &[
            entry(
                ("⇧", "^"),
                "release",
                &[],
                "",
                "Bump, tag, and optionally publish notes",
            ),
            entry(
                ("⇩", "v"),
                "update",
                &[],
                "",
                "Download and install the latest release",
            ),
            entry(
                ("⌂", "&"),
                "github",
                &[],
                "",
                "Open the shellops-cli repository",
            ),
            entry(
                ("?", "?"),
                "help",
                &[],
                "[COMMAND]",
                "This screen, or every option of one command",
            ),
        ],
    },
];

pub const FLAGS: &[Flag] = &[
    flag(
        "",
        "--secret",
        "",
        "With config set, store the value as a secret",
    ),
    flag(
        "",
        "--stdin",
        "",
        "With config set, read the value from stdin",
    ),
    flag(
        "",
        "--project",
        "",
        "With config set or unset, scope the key to this project",
    ),
    flag(
        "",
        "--color",
        "WHEN",
        "Color output: auto, always, or never",
    ),
    flag("-h", "--help", "", "Show this help"),
    flag("-V", "--version", "", "Print the version"),
];

pub const EXAMPLES: &[(&str, &str)] = &[
    ("so compose up -d", "Start the compose stack"),
    ("so cc", "Clear caches in parallel phases"),
    ("so phpstan src/Entity/Foo.php", "Analyse one PHP file"),
    ("so release", "Bump, tag, and push a release"),
    (
        "so config set github --secret --stdin",
        "Store a token without shell history",
    ),
    ("so help build", "Every option of one command"),
];

struct Row {
    cells: Vec<String>,
    about: &'static str,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Commands,
    Flags,
    Examples,
}

struct Section {
    title: &'static str,
    kind: Kind,
    rows: Vec<Row>,
}

fn command_rows(theme: Theme, group: &Group, compact: bool) -> Vec<Row> {
    let style = (group.style)();
    group
        .entries
        .iter()
        .map(|entry| {
            let icon = if theme.unicode() {
                entry.icon.0
            } else {
                entry.icon.1
            };
            let mut name = theme.paint(entry.name, style.bold());
            for alias in entry.aliases {
                name.push_str(&theme.dim(format!(", {alias}")));
            }
            let cells = if compact {
                if !entry.args.is_empty() {
                    name.push(' ');
                    name.push_str(&theme.dim(entry.args));
                }
                vec![theme.paint(icon, style), name]
            } else {
                vec![theme.paint(icon, style), name, theme.dim(entry.args)]
            };
            Row {
                cells,
                about: entry.about,
            }
        })
        .collect()
}

fn flag_rows(theme: Theme, compact: bool) -> Vec<Row> {
    FLAGS
        .iter()
        .map(|flag| {
            let mut name = if flag.short.is_empty() {
                format!("    {}", theme.flag(flag.long))
            } else {
                format!(
                    "{}{} {}",
                    theme.flag(flag.short),
                    theme.dim(","),
                    theme.flag(flag.long)
                )
            };
            let cells = if compact {
                if !flag.value.is_empty() {
                    name.push(' ');
                    name.push_str(&theme.dim(flag.value));
                }
                vec![name]
            } else {
                vec![name, theme.dim(flag.value)]
            };
            Row {
                cells,
                about: flag.about,
            }
        })
        .collect()
}

fn example_rows(theme: Theme) -> Vec<Row> {
    EXAMPLES
        .iter()
        .map(|(command, about)| Row {
            cells: vec![theme.dim("$"), highlight(theme, command)],
            about,
        })
        .collect()
}

fn sections(theme: Theme, compact: bool) -> Vec<Section> {
    let mut out: Vec<Section> = GROUPS
        .iter()
        .map(|group| Section {
            title: group.title,
            kind: Kind::Commands,
            rows: command_rows(theme, group, compact),
        })
        .collect();
    out.push(Section {
        title: "Flags",
        kind: Kind::Flags,
        rows: flag_rows(theme, compact),
    });
    out.push(Section {
        title: "Examples",
        kind: Kind::Examples,
        rows: example_rows(theme),
    });
    out
}

fn fit(sections: &[Section], kind: Kind) -> Grid {
    Grid::fit(
        INDENT,
        sections
            .iter()
            .filter(|section| section.kind == kind)
            .flat_map(|section| section.rows.iter().map(|row| row.cells.as_slice())),
    )
}

struct Layout {
    sections: Vec<Section>,
    commands: Grid,
    flags: Grid,
    examples: Grid,
    stack_examples: bool,
}

/// Wide: one argument column and one description column shared by every row. Compact (when
/// that leaves descriptions under [`MIN_ABOUT`] columns): arguments follow the command name,
/// commands and flags share a description column, and examples align among themselves.
fn layout(theme: Theme, total: usize) -> Layout {
    let sections = sections(theme, false);
    let mut commands = fit(&sections, Kind::Commands);
    let mut flags = fit(&sections, Kind::Flags);
    let mut examples = fit(&sections, Kind::Examples);
    let values = commands.column_start(2).max(flags.column_start(1));
    commands.align_column_to(2, values);
    flags.align_column_to(1, values);
    let column = [&commands, &flags, &examples]
        .iter()
        .map(|grid| grid.text_column())
        .max()
        .unwrap_or(0);
    if total.saturating_sub(column) >= MIN_ABOUT {
        for grid in [&mut commands, &mut flags, &mut examples] {
            grid.align_text_to(column);
        }
        return Layout {
            sections,
            commands,
            flags,
            examples,
            stack_examples: false,
        };
    }
    let sections = self::sections(theme, true);
    let mut commands = fit(&sections, Kind::Commands);
    let mut flags = fit(&sections, Kind::Flags);
    let mut examples = fit(&sections, Kind::Examples);
    let column = commands.text_column().max(flags.text_column());
    commands.align_text_to(column);
    flags.align_text_to(column);
    examples.align_text_to(column);
    let stack_examples = total.saturating_sub(examples.text_column()) < MIN_EXAMPLE_ABOUT;
    Layout {
        sections,
        commands,
        flags,
        examples,
        stack_examples,
    }
}

pub fn help_text(theme: Theme, total: usize) -> String {
    let Layout {
        sections,
        commands,
        flags,
        examples,
        stack_examples,
    } = layout(theme, total);
    let mut out = banner(theme, total);
    for section in &sections {
        let grid = match section.kind {
            Kind::Commands => &commands,
            Kind::Flags => &flags,
            Kind::Examples => &examples,
        };
        out.push('\n');
        out.push_str(&section_line(theme, section.title));
        out.push('\n');
        for row in &section.rows {
            if stack_examples && section.kind == Kind::Examples {
                out.push_str(&grid.row(&row.cells, ""));
                out.push_str(&wrap_indented(row.about, INDENT + 1 + 2, total, |line| {
                    theme.dim(line)
                }));
                continue;
            }
            out.push_str(
                &grid.render(&row.cells, row.about, total, |line| match section.kind {
                    Kind::Examples => theme.dim(line),
                    Kind::Commands | Kind::Flags => line.to_string(),
                }),
            );
        }
    }
    out.push('\n');
    let star = if theme.unicode() { "★" } else { "*" };
    out.push_str(&wrap_indented(
        &format!("{star} {}", crate::update::REPO_URL),
        INDENT,
        total,
        |line| match line.split_once(' ') {
            Some((icon, rest)) if icon == star => {
                format!(
                    "{} {}",
                    theme.paint(icon, Style::new().yellow()),
                    theme.paint(rest, Style::new().cyan().underline())
                )
            }
            _ => theme.paint(line, Style::new().cyan().underline()),
        },
    ));
    out
}

pub fn print_help() {
    let mut stdout = io::stdout().lock();
    let _ = write!(stdout, "{}", help_text(Theme::stdout(), term::width()));
}

fn highlight(theme: Theme, command: &str) -> String {
    command
        .split(' ')
        .enumerate()
        .map(|(index, word)| match index {
            0 => theme.paint(word, Style::new().bold().green()),
            1 => theme.command(word),
            _ if word.starts_with('-') => theme.flag(word),
            _ => theme.token(word),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::layout::{strip_ansi, width};
    use crate::ui::term::Depth;

    fn themes() -> [Theme; 4] {
        [
            Theme::plain(),
            Theme::colored(),
            Theme::colored().with_depth(Depth::TrueColor),
            Theme::colored().with_depth(Depth::Ansi256),
        ]
    }

    fn offset(line: &str, needle: &str) -> usize {
        let index = line
            .find(needle)
            .unwrap_or_else(|| panic!("{needle:?} not in {line:?}"));
        width(&line[..index])
    }

    fn first_word(text: &str) -> &str {
        text.split(' ').next().unwrap()
    }

    fn command_line<'a>(lines: &'a [&'a str], name: &str) -> &'a str {
        lines
            .iter()
            .find(|line| {
                let cells: Vec<&str> = line.split_whitespace().collect();
                cells.len() > 1 && cells[1].trim_end_matches(',') == name
            })
            .unwrap_or_else(|| panic!("no row for {name}"))
    }

    fn assert_logo_gap(text: &str) {
        let lines: Vec<&str> = text.lines().collect();
        let tag = lines
            .iter()
            .position(|line| line.contains("Docker, PHP, and release commands."))
            .expect("tagline");
        assert!(tag >= 3, "logo, two blank lines, tagline");
        assert_eq!(lines[tag - 1], "", "blank line above the tagline");
        assert_eq!(lines[tag - 2], "", "second blank line under the logo");
        let logo = lines[tag - 3];
        assert!(
            logo.contains('╚') || logo.contains('|') || logo.contains('_'),
            "logo line above the blank lines: {logo:?}"
        );
    }

    /// Column offsets of every description (`about`) and argument, grouped by section kind.
    fn columns(
        lines: &[&str],
        compact: bool,
        stacked: bool,
    ) -> (Vec<usize>, Vec<usize>, Vec<usize>) {
        let mut about = Vec::new();
        let mut args = Vec::new();
        let mut examples = Vec::new();
        for group in GROUPS {
            for entry in group.entries {
                let line = command_line(lines, entry.name);
                assert_eq!(offset(line, entry.name), INDENT + 1 + 2);
                if !entry.args.is_empty() {
                    let at = offset(line, entry.args);
                    if compact {
                        let name_end = line.find(entry.args).unwrap();
                        assert_eq!(
                            &line[name_end - 1..name_end],
                            " ",
                            "args follow the name: {line:?}"
                        );
                    } else {
                        args.push(at);
                    }
                }
                about.push(offset(line, first_word(entry.about)));
            }
        }
        for flag in FLAGS {
            let line = lines
                .iter()
                .find(|line| {
                    line.contains(&format!(" {} ", flag.long)) || line.ends_with(flag.long)
                })
                .filter(|line| line.trim_start().starts_with('-'))
                .unwrap_or_else(|| panic!("no row for {}", flag.long));
            if !flag.value.is_empty() && !compact {
                args.push(offset(line, flag.value));
            }
            about.push(offset(line, first_word(flag.about)));
        }
        for (command, text) in EXAMPLES {
            let index = lines
                .iter()
                .position(|line| line.contains(command))
                .unwrap();
            let line = if stacked {
                lines[index + 1]
            } else {
                lines[index]
            };
            examples.push(offset(line, first_word(text)));
        }
        (about, args, examples)
    }

    fn same(columns: &[usize]) -> bool {
        columns.iter().all(|column| *column == columns[0])
    }

    #[test]
    fn every_row_shares_one_description_column() {
        for total in [60, 80, 100, 120] {
            let plain = help_text(Theme::plain(), total);
            if total >= 80 {
                assert_logo_gap(&plain);
            }
            let compact = total < 100;
            let stacked = total < 70;
            for theme in themes() {
                let text = strip_ansi(&help_text(theme, total));
                assert_eq!(text, plain, "colors changed the layout at {total}");
                let lines: Vec<&str> = text.lines().collect();
                let (about, args, examples) = columns(&lines, compact, stacked);
                assert!(same(&about), "{total}: {about:?}");
                assert!(same(&args), "{total}: {args:?}");
                assert!(same(&examples), "{total}: {examples:?}");
                if stacked {
                    assert_eq!(examples[0], INDENT + 3, "stacked under the command");
                }
                if !compact {
                    assert_eq!(about[0], examples[0], "wide layout shares one column");
                }
                assert!(
                    total - about[0] >= MIN_EXAMPLE_ABOUT,
                    "descriptions keep room at {total}: column {}",
                    about[0]
                );
                for line in &lines {
                    assert!(width(line) <= total, "{} > {total}: {line:?}", width(line));
                }
                let grid_start = lines
                    .iter()
                    .position(|line| line.starts_with("==> "))
                    .unwrap();
                let mut in_examples = false;
                for line in &lines[grid_start..] {
                    if line.starts_with("==> ") {
                        in_examples = *line == "==> Examples";
                    }
                    let lead = line.len() - line.trim_start().len();
                    if lead > INDENT && !line.trim_start().starts_with("--") {
                        let expected = if in_examples { examples[0] } else { about[0] };
                        assert_eq!(lead, expected, "ragged continuation: {line:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn narrow_help_moves_arguments_next_to_the_name() {
        let text = help_text(Theme::plain(), 80);
        assert!(text.contains("config set|get|list|unset"), "{text}");
        assert!(text.contains("--color WHEN"), "{text}");
        let wide = help_text(Theme::plain(), 120);
        assert!(!wide.contains("config set|get|list|unset"));
    }

    #[test]
    fn sections_are_separated_by_one_blank_line() {
        let text = help_text(Theme::plain(), 100);
        let lines: Vec<&str> = text.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            if line.starts_with("==> ") {
                assert_eq!(lines[index - 1], "", "{line}");
                assert_ne!(lines[index - 2], "", "{line}");
            }
        }
        for title in [
            "Compose", "PHP", "Data", "Project", "Release", "Flags", "Examples",
        ] {
            assert!(lines.contains(&format!("==> {title}").as_str()), "{title}");
        }
        let tagline = text.find("Docker, PHP, and release commands.").unwrap();
        assert!(!text[tagline..].contains("\n\n\n"));
    }

    #[test]
    fn help_lists_every_command_alias_and_flag() {
        let text = help_text(Theme::plain(), 100);
        for group in GROUPS {
            for entry in group.entries {
                assert!(text.contains(entry.about), "{}", entry.name);
                for alias in entry.aliases {
                    assert!(text.contains(&format!("{}, {alias}", entry.name)));
                }
            }
        }
        for flag in FLAGS {
            assert!(text.contains(flag.long), "{}", flag.long);
        }
        assert!(text.contains(crate::update::REPO_URL));
        assert!(!text.contains('\u{1b}'));
        assert!(help_text(Theme::colored(), 100).contains('\u{1b}'));
    }

    #[test]
    fn ascii_help_keeps_the_grid() {
        let text = help_text(Theme::ascii(), 80);
        assert_logo_gap(&text);
        assert!(text.is_ascii());
        let lines: Vec<&str> = text.lines().collect();
        let columns: Vec<usize> = GROUPS
            .iter()
            .flat_map(|group| group.entries.iter())
            .map(|entry| offset(command_line(&lines, entry.name), first_word(entry.about)))
            .collect();
        assert!(columns.iter().all(|column| *column == columns[0]));
    }
}
