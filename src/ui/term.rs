use std::io::{self, IsTerminal};
use std::sync::OnceLock;

pub const FALLBACK_WIDTH: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum ColorMode {
    #[default]
    Auto,
    Always,
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Depth {
    Basic,
    Ansi256,
    TrueColor,
}

#[derive(Debug, Clone, Default)]
pub struct Env {
    pub no_color: Option<String>,
    pub clicolor_force: Option<String>,
    pub force_color: Option<String>,
    pub clicolor: Option<String>,
    pub term: Option<String>,
    pub colorterm: Option<String>,
    pub term_program: Option<String>,
    pub locale: Option<String>,
}

impl Env {
    pub fn current() -> Self {
        let var = |key: &str| std::env::var(key).ok();
        Self {
            no_color: var("NO_COLOR"),
            clicolor_force: var("CLICOLOR_FORCE"),
            force_color: var("FORCE_COLOR"),
            clicolor: var("CLICOLOR"),
            term: var("TERM"),
            colorterm: var("COLORTERM"),
            term_program: var("TERM_PROGRAM"),
            locale: ["LC_ALL", "LC_CTYPE", "LANG"]
                .into_iter()
                .filter_map(var)
                .find(|value| !value.is_empty()),
        }
    }
}

pub fn color_enabled(mode: ColorMode, env: &Env, tty: bool) -> bool {
    match mode {
        ColorMode::Always => return true,
        ColorMode::Never => return false,
        ColorMode::Auto => {}
    }
    if env
        .no_color
        .as_deref()
        .is_some_and(|value| !value.is_empty())
    {
        return false;
    }
    if env
        .clicolor_force
        .as_deref()
        .is_some_and(|value| !value.is_empty() && value != "0")
    {
        return true;
    }
    if let Some(force) = env.force_color.as_deref() {
        return !matches!(force, "0" | "false");
    }
    if env.clicolor.as_deref() == Some("0") || env.term.as_deref() == Some("dumb") {
        return false;
    }
    tty
}

pub fn color_depth(env: &Env) -> Depth {
    match env.force_color.as_deref() {
        Some("3") => return Depth::TrueColor,
        Some("2") => return Depth::Ansi256,
        _ => {}
    }
    let colorterm = env.colorterm.as_deref().unwrap_or("").to_ascii_lowercase();
    if colorterm == "truecolor" || colorterm == "24bit" {
        return Depth::TrueColor;
    }
    let program = env.term_program.as_deref().unwrap_or("");
    if matches!(program, "iTerm.app" | "vscode" | "WezTerm" | "ghostty") {
        return Depth::TrueColor;
    }
    let term = env.term.as_deref().unwrap_or("").to_ascii_lowercase();
    if term.contains("truecolor") || term.contains("24bit") || term.ends_with("-direct") {
        Depth::TrueColor
    } else if term.contains("256") || program == "Apple_Terminal" {
        Depth::Ansi256
    } else {
        Depth::Basic
    }
}

pub fn unicode_locale(env: &Env) -> bool {
    match env.locale.as_deref() {
        None => true,
        Some(locale) => {
            let locale = locale.to_ascii_lowercase();
            locale.contains("utf-8") || locale.contains("utf8")
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub stdout: bool,
    pub stderr: bool,
    pub depth: Depth,
    pub unicode: bool,
}

static SETTINGS: OnceLock<Settings> = OnceLock::new();

fn detect(mode: ColorMode) -> Settings {
    let env = Env::current();
    Settings {
        stdout: color_enabled(mode, &env, io::stdout().is_terminal()),
        stderr: color_enabled(mode, &env, io::stderr().is_terminal()),
        depth: color_depth(&env),
        unicode: unicode_locale(&env),
    }
}

fn apply(settings: Settings) -> Settings {
    console::set_colors_enabled(settings.stdout);
    console::set_colors_enabled_stderr(settings.stderr);
    settings
}

pub fn init(mode: ColorMode) -> Settings {
    *SETTINGS.get_or_init(|| apply(detect(mode)))
}

pub fn settings() -> Settings {
    init(ColorMode::Auto)
}

pub fn width() -> usize {
    console::Term::stdout()
        .size_checked()
        .map(|(_, cols)| cols as usize)
        .filter(|cols| *cols > 0)
        .or_else(|| {
            std::env::var("COLUMNS")
                .ok()
                .and_then(|value| value.parse().ok())
        })
        .unwrap_or(FALLBACK_WIDTH)
}

pub fn mode_from_args<I, S>(args: I) -> ColorMode
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    use clap::ValueEnum;
    let mut mode = ColorMode::Auto;
    let mut expect_value = false;
    for arg in args {
        let Some(arg) = arg.as_ref().to_str() else {
            expect_value = false;
            continue;
        };
        let value = if expect_value {
            expect_value = false;
            Some(arg)
        } else if arg == "--" {
            break;
        } else if arg == "--color" {
            expect_value = true;
            None
        } else {
            arg.strip_prefix("--color=")
        };
        if let Some(parsed) = value.and_then(|value| ColorMode::from_str(value, true).ok()) {
            mode = parsed;
        }
    }
    mode
}
