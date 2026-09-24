use anyhow::{Context, Result, bail};
use clap::builder::styling::{AnsiColor, Effects, Styles};
use clap::{ArgAction, Args, ColorChoice, CommandFactory, FromArgMatches, Parser, Subcommand};
use std::io::{self, Read};
use std::path::PathBuf;

use crate::config::{self, Store};
use crate::php;
use crate::project;
use crate::ui::{self, term::ColorMode};

fn styles() -> Styles {
    Styles::styled()
        .header(AnsiColor::Blue.on_default().effects(Effects::BOLD))
        .usage(AnsiColor::Blue.on_default().effects(Effects::BOLD))
        .literal(AnsiColor::Cyan.on_default().effects(Effects::BOLD))
        .placeholder(AnsiColor::BrightBlack.on_default())
        .error(AnsiColor::Red.on_default().effects(Effects::BOLD))
        .valid(AnsiColor::Green.on_default())
        .invalid(AnsiColor::Yellow.on_default().effects(Effects::BOLD))
}

#[derive(Parser)]
#[command(
    name = "so",
    version,
    about = "Docker, PHP, and release commands",
    styles = styles(),
    disable_help_subcommand = true,
    disable_help_flag = true
)]
pub struct Cli {
    #[arg(short = 'h', long = "help", action = ArgAction::SetTrue)]
    pub help: bool,
    #[arg(long, global = true, value_enum, value_name = "WHEN", default_value_t = ColorMode::Auto)]
    pub color: ColorMode,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Run docker compose.
    Compose(Passthrough),
    /// Build local images from *.Dockerfile files.
    Build,
    /// Clear caches in three phases.
    Cc,
    /// Run bin/console. Alias: c.
    #[command(alias = "c")]
    Console(Passthrough),
    /// Run Composer. Alias: cp.
    #[command(alias = "cp")]
    Composer(Passthrough),
    /// composer dump-autoload.
    Dump,
    /// PHP CS Fixer. Alias: csfixer.
    #[command(alias = "csfixer")]
    Csf,
    /// PHPStan.
    Phpstan(Passthrough),
    /// Phan.
    Phan(Passthrough),
    /// PHPUnit.
    Phpunit(Passthrough),
    /// Rector.
    Rector(Passthrough),
    /// PHPMD.
    Phpmd,
    /// Load fixtures. Alias: f.
    #[command(alias = "f")]
    Fixtures,
    /// Drop, migrate, and load fixtures. Alias: rd.
    #[command(alias = "rd")]
    ResetDatabase,
    /// Force-drop the Postgres database. Alias: fdd.
    #[command(alias = "fdd")]
    ForceDropDatabase,
    /// Install dependencies and rebuild the database.
    Installer,
    /// Yarn in the vue service.
    Yarn(Passthrough),
    /// Show the detected project.
    Debug,
    /// Sort a dotenv file in place.
    SortDotenv { path: PathBuf },
    /// Generate an OpenSSL key pair.
    Keypair {
        path: PathBuf,
        suffix: Option<String>,
    },
    /// Print a random alphanumeric string.
    Randomstr { length: usize },
    /// Read and write ~/.shellops settings.
    Config(ConfigArgs),
    /// Bump versions, tag, and push a release.
    Release,
    /// Download and install the latest release.
    Update,
    /// Open the GitHub repository.
    Github,
    /// Show help.
    Help(HelpArgs),
}

#[derive(Args, Debug, Clone)]
pub struct Passthrough {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub args: Vec<String>,
}

#[derive(Args, Debug, Clone)]
pub struct HelpArgs {
    pub command: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: ConfigCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum ConfigCommand {
    /// Set a key.
    Set(SetArgs),
    /// Print a plain value.
    Get { key: String },
    /// List keys. Secrets are masked.
    List,
    /// Remove a key.
    Unset(UnsetArgs),
}

#[derive(Args, Debug, Clone)]
pub struct SetArgs {
    pub key: String,
    pub value: Option<String>,
    #[arg(long)]
    pub secret: bool,
    #[arg(long)]
    pub stdin: bool,
    #[arg(long)]
    pub project: bool,
}

#[derive(Args, Debug, Clone)]
pub struct UnsetArgs {
    pub key: String,
    #[arg(long)]
    pub project: bool,
}

pub fn run() -> Result<i32> {
    let settings = ui::term::init(ui::term::mode_from_args(std::env::args_os()));
    let matches = Cli::command()
        .color(clap_color(settings.stderr))
        .get_matches();
    let cli = Cli::from_arg_matches(&matches).unwrap_or_else(|err| err.exit());
    dispatch(cli)
}

fn clap_color(enabled: bool) -> ColorChoice {
    if enabled {
        ColorChoice::Always
    } else {
        ColorChoice::Never
    }
}

fn dispatch(cli: Cli) -> Result<i32> {
    let command = match cli.command {
        Some(command) if !cli.help => command,
        _ => {
            crate::update::notify_if_outdated();
            ui::print_help();
            return Ok(0);
        }
    };
    if matches!(command, Command::Update) {
        crate::update::run_update()?;
        return Ok(0);
    }
    crate::update::notify_if_outdated();
    match command {
        Command::Github => {
            crate::update::open_github()?;
            Ok(0)
        }
        Command::Help(args) => show_help(args.command.as_deref()),
        Command::Update => Ok(0),
        Command::Config(args) => config_command(args),
        Command::Release => crate::release::run(),
        Command::SortDotenv { path } => crate::tools::sort_dotenv(&path),
        Command::Keypair { path, suffix } => crate::tools::keypair(&path, suffix.as_deref()),
        Command::Randomstr { length } => crate::tools::random_string(length),
        other => {
            let project = project::discover()?;
            match other {
                Command::Compose(args) => crate::compose::compose(&project, &args.args),
                Command::Build => crate::build::build(&project),
                Command::Cc => crate::cache::clear(&project),
                Command::Console(args) => php::run_console(&project, &args.args),
                Command::Composer(args) => php::run_composer(&project, &args.args),
                Command::Dump => php::run_composer(&project, &["dump-autoload".into()]),
                Command::Csf => php::run_tool(&project, &php::csf_args()),
                Command::Phpstan(args) => php::run_tool(&project, &php::phpstan_args(&args.args)),
                Command::Phan(args) => {
                    php::run_tool(&project, &php::phan_args(&project.root, &args.args)?)
                }
                Command::Phpunit(args) => php::run_tool(&project, &php::phpunit_args(&args.args)),
                Command::Rector(args) => php::run_tool(&project, &php::rector_args(&args.args)),
                Command::Phpmd => php::run_tool(&project, &php::phpmd_args()),
                Command::Fixtures => php::run_fixtures(&project),
                Command::ResetDatabase => php::reset_database(&project),
                Command::ForceDropDatabase => php::force_drop_database(&project),
                Command::Installer => php::installer(&project),
                Command::Yarn(args) => php::run_tool(&project, &php::yarn_args(&args.args)),
                Command::Debug => crate::tools::debug(&project),
                Command::Config(_)
                | Command::Release
                | Command::Update
                | Command::Github
                | Command::Help(_)
                | Command::SortDotenv { .. }
                | Command::Keypair { .. }
                | Command::Randomstr { .. } => Ok(0),
            }
        }
    }
}

fn show_help(command: Option<&str>) -> Result<i32> {
    let Some(name) = command else {
        ui::print_help();
        return Ok(0);
    };
    let mut root = Cli::command().color(clap_color(ui::term::settings().stdout));
    root.build();
    let Some(sub) = root.find_subcommand_mut(name) else {
        return Err(ui::hinted(
            format!("unknown command: {name}"),
            "Run so help to see every command.",
        ));
    };
    sub.print_help()?;
    println!();
    Ok(0)
}

fn config_command(args: ConfigArgs) -> Result<i32> {
    let store = Store::open()?;
    match args.command {
        ConfigCommand::Set(args) => {
            let project = project_scope(args.project)?;
            let value = config_value(args.value, args.stdin)?;
            store.set(&args.key, &value, args.secret, project.as_deref())?;
            ui::ok(&format!("set {}", args.key));
            Ok(0)
        }
        ConfigCommand::Get { key } => {
            let project = project::git_root().ok();
            let hit = store.get(&key, project.as_deref())?;
            println!("{}", hit.value);
            Ok(0)
        }
        ConfigCommand::List => {
            let project = project::git_root().ok();
            let rows = store.list(project.as_deref())?;
            crate::tools::config_list(&rows);
            Ok(0)
        }
        ConfigCommand::Unset(args) => {
            let project = project_scope(args.project)?;
            store.unset(&args.key, project.as_deref())?;
            ui::ok(&format!("unset {}", args.key));
            Ok(0)
        }
    }
}

fn project_scope(project: bool) -> Result<Option<PathBuf>> {
    if !project {
        return Ok(None);
    }
    Ok(Some(config::project_key()?))
}

fn config_value(value: Option<String>, stdin: bool) -> Result<String> {
    if stdin && value.is_some() {
        bail!("pass a value or --stdin, not both");
    }
    if stdin {
        let mut text = String::new();
        io::stdin().read_to_string(&mut text)?;
        if text.ends_with('\n') {
            text.pop();
            if text.ends_with('\r') {
                text.pop();
            }
        }
        if text.is_empty() {
            bail!("stdin was empty");
        }
        return Ok(text);
    }
    value.context("a value or --stdin is required")
}
