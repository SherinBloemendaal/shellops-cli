use anyhow::{Context, Result, bail};
use clap::builder::styling::{AnsiColor, Effects, Styles};
use clap::{ArgAction, Args, ColorChoice, CommandFactory, FromArgMatches, Parser, Subcommand};
use std::ffi::OsString;
use std::io::{self, IsTerminal, Read};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::config::{self, Store};
use crate::php;
use crate::project;
use crate::ui::{self, term::ColorMode};

fn styles() -> Styles {
    Styles::styled()
        .header(AnsiColor::Green.on_default().effects(Effects::BOLD))
        .usage(AnsiColor::Green.on_default().effects(Effects::BOLD))
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
    /// Color output: auto, always, or never.
    #[arg(
        long,
        global = true,
        value_enum,
        value_name = "WHEN",
        default_value_t = ColorMode::Auto,
        hide_default_value = true,
        hide_possible_values = true,
        display_order = 100
    )]
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
    SortDotenv {
        /// The dotenv file to sort.
        path: PathBuf,
    },
    /// Generate an OpenSSL key pair.
    Keypair {
        /// Directory that receives private.pem and public.pem.
        path: PathBuf,
        /// Appended to the file names, as in private-SUFFIX.pem.
        suffix: Option<String>,
    },
    /// Print a random alphanumeric string.
    Randomstr {
        /// Number of characters.
        length: usize,
    },
    /// Read and write ~/.shellops settings.
    Config(ConfigArgs),
    /// Bump versions, tag, and push a release.
    Release,
    /// Download and install the latest release.
    Update,
    /// Open the GitHub repository.
    Github,
    /// Show help, or every option of one command.
    Help(HelpArgs),
    /// Set up identity, API keys, and local checks.
    Init(InitArgs),
    /// Create and run signed command aliases.
    Alias(AliasArgs),
    /// Run a user alias.
    #[command(external_subcommand)]
    External(Vec<OsString>),
}

#[derive(Args, Debug, Clone)]
pub struct Passthrough {
    /// Passed to the tool unchanged, flags included.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub args: Vec<String>,
}

#[derive(Args, Debug, Clone)]
pub struct HelpArgs {
    /// A command or alias to explain.
    pub command: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct InitArgs {
    /// One step: identity, openrouter, github, docker, or status.
    pub step: Option<String>,
    /// Show which steps are done instead of running them.
    #[arg(long)]
    pub status: bool,
    /// With status, verify each finished step online.
    #[arg(long)]
    pub check: bool,
}

#[derive(Args, Debug, Clone)]
pub struct AliasArgs {
    #[command(subcommand)]
    pub command: AliasCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum AliasCommand {
    /// Create an alias. The body is one shell string.
    Create(AliasWrite),
    /// Edit an alias and sign it again.
    Edit(AliasWrite),
    /// Rename an alias without changing its signature.
    Rename(AliasRename),
    /// Remove an alias.
    Rm(AliasName),
    /// Show an alias, its author, and whether it can run.
    View(AliasLookup),
    /// Trust an alias body, or its author with --author.
    Trust(AliasLookup),
    /// Remove a content or author trust.
    Untrust(AliasLookup),
    /// List directory and global aliases.
    List,
}

#[derive(Args, Debug, Clone)]
pub struct AliasWrite {
    /// The alias in ~/.shellops, available everywhere.
    #[arg(long)]
    pub global: bool,
    /// The alias stored in this repository.
    #[arg(long)]
    pub dir: bool,
    /// Alias name: lowercase letters, digits, and dashes.
    pub name: String,
    /// Shell string to run. Edit opens $EDITOR without it.
    pub body: Option<String>,
    /// Read the body from stdin.
    #[arg(long)]
    pub stdin: bool,
}

#[derive(Args, Debug, Clone)]
pub struct AliasRename {
    /// The alias in ~/.shellops, available everywhere.
    #[arg(long)]
    pub global: bool,
    /// The alias stored in this repository.
    #[arg(long)]
    pub dir: bool,
    /// Current name.
    pub old: String,
    /// New name.
    pub new: String,
}

#[derive(Args, Debug, Clone)]
pub struct AliasName {
    /// The alias in ~/.shellops, available everywhere.
    #[arg(long)]
    pub global: bool,
    /// The alias stored in this repository.
    #[arg(long)]
    pub dir: bool,
    /// Alias name.
    pub name: String,
}

#[derive(Args, Debug, Clone)]
pub struct AliasLookup {
    /// The alias in ~/.shellops, available everywhere.
    #[arg(long)]
    pub global: bool,
    /// The alias stored in this repository.
    #[arg(long)]
    pub dir: bool,
    /// Alias name.
    pub name: String,
    /// Apply to the author's key instead of these exact contents.
    #[arg(long)]
    pub author: bool,
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
    Get {
        /// Key to print, such as openrouter.model.
        key: String,
    },
    /// List keys. Secrets are masked.
    List,
    /// Remove a key.
    Unset(UnsetArgs),
}

#[derive(Args, Debug, Clone)]
pub struct SetArgs {
    /// Key to set, such as openrouter.model.
    pub key: String,
    /// The value. Omit it with --stdin.
    pub value: Option<String>,
    /// Store as a secret: in secrets.toml, masked in lists.
    #[arg(long)]
    pub secret: bool,
    /// Read the value from stdin, out of shell history.
    #[arg(long)]
    pub stdin: bool,
    /// Scope the key to this project.
    #[arg(long)]
    pub project: bool,
}

#[derive(Args, Debug, Clone)]
pub struct UnsetArgs {
    /// Key to remove.
    pub key: String,
    /// Remove the project-scoped value.
    #[arg(long)]
    pub project: bool,
}

/// Subcommands that hand every argument, `--help` included, to the tool they wrap.
const PASSTHROUGH: &[&str] = &[
    "compose", "console", "composer", "phpstan", "phan", "phpunit", "rector", "yarn",
];

/// The root `-h` prints the custom help screen; every other command gets clap's `-h/--help`,
/// except passthrough commands, whose `--help` belongs to the wrapped tool.
pub fn command() -> clap::Command {
    // DisableHelpFlag is a global setting, so the flag is added back explicitly.
    fn with_help(command: clap::Command) -> clap::Command {
        command
            .arg(
                clap::Arg::new("help")
                    .short('h')
                    .long("help")
                    .action(ArgAction::Help)
                    .display_order(101)
                    .help("Print help"),
            )
            .mut_subcommands(with_help)
    }
    Cli::command().mut_subcommands(|sub| {
        if PASSTHROUGH.contains(&sub.get_name()) {
            sub
        } else {
            with_help(sub)
        }
    })
}

pub fn run() -> Result<i32> {
    ui::signal::install();
    let settings = ui::term::init(ui::term::mode_from_args(std::env::args_os()));
    let matches = match command()
        .color(clap_color(settings.stderr))
        .try_get_matches()
    {
        Ok(matches) => matches,
        Err(err) => return Ok(parse_error(err)),
    };
    let cli = match Cli::from_arg_matches(&matches) {
        Ok(cli) => cli,
        Err(err) => return Ok(parse_error(err)),
    };
    dispatch(cli)
}

/// Help and version requests print as clap renders them; real parse errors use the same
/// `✖` block as every other ShellOps error.
fn parse_error(err: clap::Error) -> i32 {
    use clap::error::ErrorKind;
    match err.kind() {
        ErrorKind::DisplayHelp
        | ErrorKind::DisplayVersion
        | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
            let settings = ui::term::settings();
            let color = if err.use_stderr() {
                settings.stderr
            } else {
                settings.stdout
            };
            let text = styled(&err.render(), color);
            if err.use_stderr() {
                eprint!("{}", ui::wrap_help(&text, ui::term::stderr_width()));
            } else {
                print_clap_help(&text, ui::term::width());
            }
            err.exit_code()
        }
        _ => {
            eprintln!(
                "{}",
                ui::clap_error_text(
                    ui::Theme::stderr(),
                    &err.render().to_string(),
                    ui::term::stderr_width()
                )
            );
            err.exit_code()
        }
    }
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
            let outdated = crate::update::notify_if_outdated();
            let pending = crate::setup::notify_if_pending();
            if outdated || pending {
                println!();
            }
            print_full_help();
            return Ok(0);
        }
    };
    if matches!(command, Command::Update) {
        crate::update::run_update()?;
        return Ok(0);
    }
    if !matches!(command, Command::Init(_)) {
        crate::update::notify_if_outdated();
        crate::setup::notify_if_pending();
    }
    match command {
        Command::Github => {
            crate::update::open_github()?;
            Ok(0)
        }
        Command::Help(args) => show_help(args.command.as_deref()),
        Command::Init(args) => crate::setup::run(args.step.as_deref(), args.status, args.check),
        Command::Alias(args) => alias_command(args),
        Command::External(args) => reported(|| external_command(args)),
        Command::Update => Ok(0),
        Command::Config(args) => config_command(args),
        Command::Release => crate::release::run(),
        Command::SortDotenv { path } => crate::tools::sort_dotenv(&path),
        Command::Keypair { path, suffix } => crate::tools::keypair(&path, suffix.as_deref()),
        Command::Randomstr { length } => crate::tools::random_string(length),
        other => {
            let project = project::discover()?;
            match other {
                Command::Compose(args) => {
                    reported(|| crate::compose::compose(&project, &args.args))
                }
                Command::Build => crate::build::build(&project),
                Command::Cc => crate::cache::clear(&project),
                Command::Console(args) => reported(|| php::run_console(&project, &args.args)),
                Command::Composer(args) => reported(|| php::run_composer(&project, &args.args)),
                Command::Dump => {
                    reported(|| php::run_composer(&project, &["dump-autoload".into()]))
                }
                Command::Csf => reported(|| php::run_tool(&project, &php::csf_args())),
                Command::Phpstan(args) => {
                    reported(|| php::run_tool(&project, &php::phpstan_args(&args.args)))
                }
                Command::Phan(args) => {
                    let tool = php::phan_args(&project.root, &args.args)?;
                    reported(|| php::run_tool(&project, &tool))
                }
                Command::Phpunit(args) => {
                    reported(|| php::run_tool(&project, &php::phpunit_args(&args.args)))
                }
                Command::Rector(args) => {
                    reported(|| php::run_tool(&project, &php::rector_args(&args.args)))
                }
                Command::Phpmd => reported(|| php::run_tool(&project, &php::phpmd_args())),
                Command::Fixtures => reported(|| php::run_fixtures(&project)),
                Command::ResetDatabase => php::reset_database(&project),
                Command::ForceDropDatabase => reported(|| php::force_drop_database(&project)),
                Command::Installer => php::installer(&project),
                Command::Yarn(args) => {
                    reported(|| php::run_tool(&project, &php::yarn_args(&args.args)))
                }
                Command::Debug => crate::tools::debug(&project),
                Command::Config(_)
                | Command::Release
                | Command::Update
                | Command::Github
                | Command::Help(_)
                | Command::Init(_)
                | Command::Alias(_)
                | Command::External(_)
                | Command::SortDotenv { .. }
                | Command::Keypair { .. }
                | Command::Randomstr { .. } => Ok(0),
            }
        }
    }
}

/// Successful runs this short get no closing line: their own output says enough.
const REPORT_AFTER: Duration = Duration::from_secs(2);

/// Runs a command that hands the terminal to Docker, PHP, Yarn or an alias script, then says
/// how it ended when that is worth a line: `✖ so phpunit exited 1  4.2s` after a failure, or
/// `✔ so compose up -d  12s` after a long run. The wrapped tool draws its own progress, so
/// nothing animates while it runs. Nothing is added when stderr is not a terminal, or inside
/// an alias, whose own closing line already covers it.
fn reported(run: impl FnOnce() -> Result<i32>) -> Result<i32> {
    let started = Instant::now();
    let result = run();
    if let Ok(code) = &result
        && io::stderr().is_terminal()
        && std::env::var_os("SHELLOPS_ALIAS_DEPTH").is_none()
        && let Some(line) = closing_line(
            ui::Theme::stderr(),
            &invocation(std::env::args().skip(1)),
            *code,
            started.elapsed(),
            ui::term::stderr_width(),
        )
    {
        eprintln!("{line}");
    }
    result
}

/// `so` plus the arguments it was given: how the closing line names the command.
fn invocation(args: impl Iterator<Item = String>) -> String {
    std::iter::once("so".to_string())
        .chain(args)
        .collect::<Vec<_>>()
        .join(" ")
}

/// The closing line for a passthrough run that exited with `code` after `elapsed`, if any.
pub fn closing_line(
    theme: ui::Theme,
    label: &str,
    code: i32,
    elapsed: Duration,
    width: usize,
) -> Option<String> {
    let label = ui::layout::keep_head(
        label,
        width.saturating_sub(CLOSING_CHROME).max(16),
        theme.icons().ellipsis,
    );
    match code {
        0 if elapsed < REPORT_AFTER => None,
        0 => Some(ui::done_line(theme, true, &label, elapsed)),
        code => Some(ui::done_line(
            theme,
            false,
            &format!("{label} exited {code}"),
            elapsed,
        )),
    }
}

/// Columns a closing line spends besides the command: glyph, `exited NNN`, gap and time.
const CLOSING_CHROME: usize = 2 + 11 + 2 + 7;

/// clap's rendered text, with its styles only when color is on.
fn styled(text: &clap::builder::StyledStr, color: bool) -> String {
    if color {
        text.ansi().to_string()
    } else {
        text.to_string()
    }
}

fn print_clap_help(text: &str, total: usize) {
    use std::io::Write;
    let mut stdout = io::stdout().lock();
    let _ = write!(stdout, "{}", ui::wrap_help(text, total));
}

fn print_full_help() {
    ui::print_help();
    if let Ok(roots) = alias_roots() {
        let store = crate::config::Store::open().ok();
        if let Some(store) = store
            && let Ok(section) = crate::alias::help_section(&roots, &store)
            && !section.is_empty()
        {
            print!("\n{section}");
        }
    }
}

fn show_help(command: Option<&str>) -> Result<i32> {
    let Some(name) = command else {
        print_full_help();
        return Ok(0);
    };
    let mut root = self::command().color(clap_color(ui::term::settings().stdout));
    root.build();
    let Some(sub) = root.find_subcommand_mut(name) else {
        if let Ok(roots) = alias_roots() {
            let store = crate::config::Store::open()?;
            if let Ok(records) = crate::alias::view_records(&roots, name, None) {
                for record in &records {
                    crate::alias::print_record(&store, record)?;
                }
                return Ok(0);
            }
        }
        return Err(ui::hinted(
            format!("unknown command: {name}"),
            "Run so help to see every command.",
        ));
    };
    let text = sub.render_help();
    print_clap_help(
        &styled(&text, ui::term::settings().stdout),
        ui::term::width(),
    );
    Ok(0)
}

fn alias_roots() -> Result<crate::alias::Roots> {
    let cwd = std::env::current_dir().context("could not read the working directory")?;
    let dir = project::git_toplevel(&cwd).unwrap_or(cwd);
    Ok(crate::alias::Roots {
        home: project::shellops_home()?,
        dir,
    })
}

fn alias_command(args: AliasArgs) -> Result<i32> {
    let roots = alias_roots()?;
    let store = Store::open()?;
    let signer = crate::identity::Signer::system();
    match args.command {
        AliasCommand::Create(args) => {
            let scope = crate::alias::require_scope(args.global, args.dir)?;
            let body = alias_body(args.body, args.stdin, true)?
                .context("pass the alias body, or --stdin")?;
            let record = crate::alias::create(&roots, &store, &signer, scope, &args.name, &body)?;
            ui::ok(&format!(
                "Created {} alias {}",
                record.scope.label(),
                record.file.name
            ));
            Ok(0)
        }
        AliasCommand::Edit(args) => {
            let scope = crate::alias::require_scope(args.global, args.dir)?;
            let current = crate::alias::view_records(&roots, &args.name, Some(scope))?;
            let body = match alias_body(args.body, args.stdin, false)? {
                Some(body) => body,
                None => edit_alias_body(&current[0].file.body)?,
            };
            if body == current[0].file.body {
                ui::info("Left unchanged");
                return Ok(0);
            }
            ui::section("Current");
            println!("{}", current[0].file.body.trim_end());
            ui::section("New");
            println!("{}", body.trim_end());
            if !ui::confirm_default("Sign and save this alias?", true)? {
                ui::info("Left unchanged");
                return Ok(0);
            }
            let record = crate::alias::edit(&roots, &store, &signer, scope, &args.name, &body)?;
            ui::ok(&format!(
                "Saved {} alias {}",
                record.scope.label(),
                record.file.name
            ));
            Ok(0)
        }
        AliasCommand::Rename(args) => {
            let scope = crate::alias::require_scope(args.global, args.dir)?;
            let record = crate::alias::rename(&roots, scope, &args.old, &args.new)?;
            ui::ok(&format!("Renamed to {}", record.file.name));
            Ok(0)
        }
        AliasCommand::Rm(args) => {
            let scope = crate::alias::require_scope(args.global, args.dir)?;
            let path = crate::alias::remove(&roots, scope, &args.name)?;
            ui::ok(&format!("Removed {}", ui::tilde(&path)));
            Ok(0)
        }
        AliasCommand::View(args) => {
            let scope = optional_scope(args.global, args.dir)?;
            let records = crate::alias::view_records(&roots, &args.name, scope)?;
            if records.len() > 1 {
                ui::warn("This name exists in both scopes, so it will not run.");
            }
            for record in &records {
                crate::alias::print_record(&store, record)?;
            }
            Ok(0)
        }
        AliasCommand::Trust(args) => {
            let scope = optional_scope(args.global, args.dir)?;
            let record = if args.author {
                crate::alias::trust_author(&roots, &store, &args.name, scope)?
            } else {
                crate::alias::trust_hash(&roots, &store, &args.name, scope)?
            };
            crate::alias::print_record(&store, &record)?;
            ui::ok(if args.author {
                "Trusted the author"
            } else {
                "Trusted these contents"
            });
            Ok(0)
        }
        AliasCommand::Untrust(args) => {
            let scope = optional_scope(args.global, args.dir)?;
            if args.author {
                crate::alias::untrust_author(&roots, &store, &args.name, scope)?;
                ui::ok("Forgot the author");
            } else {
                crate::alias::untrust_hash(&roots, &store, &args.name, scope)?;
                ui::ok("Forgot these contents");
            }
            Ok(0)
        }
        AliasCommand::List => {
            let records = crate::alias::list_records(&roots)?;
            if records.is_empty() {
                ui::info("No aliases yet");
                ui::hint("Create one with so alias create --global NAME 'COMMAND'.");
                return Ok(0);
            }
            print!("{}", crate::alias::help_section(&roots, &store)?);
            Ok(0)
        }
    }
}

fn external_command(args: Vec<OsString>) -> Result<i32> {
    let mut parts = args.into_iter();
    let name = parts
        .next()
        .and_then(|part| part.into_string().ok())
        .context("missing alias name")?;
    let rest = parts
        .map(|part| {
            part.into_string()
                .map_err(|_| anyhow::anyhow!("alias arguments must be UTF-8"))
        })
        .collect::<Result<Vec<_>>>()?;
    let roots = alias_roots()?;
    let store = Store::open()?;
    let depth = std::env::var("SHELLOPS_ALIAS_DEPTH")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let plan = crate::alias::plan_run(&roots, &store, &name, &rest, depth)?;
    crate::alias::execute(&plan)
}

fn optional_scope(global: bool, dir: bool) -> Result<Option<crate::alias::Scope>> {
    match (global, dir) {
        (false, false) => Ok(None),
        _ => Ok(Some(crate::alias::require_scope(global, dir)?)),
    }
}

fn alias_body(body: Option<String>, stdin: bool, required: bool) -> Result<Option<String>> {
    if stdin && body.is_some() {
        bail!("pass a body or --stdin, not both");
    }
    if stdin {
        return Ok(Some(config_value(None, true)?));
    }
    if let Some(body) = body {
        return Ok(Some(body));
    }
    if required {
        bail!("pass the alias body, or --stdin");
    }
    Ok(None)
}

fn edit_alias_body(current: &str) -> Result<String> {
    let mut file = tempfile::NamedTempFile::new().context("could not create an alias file")?;
    std::io::Write::write_all(file.as_file_mut(), current.as_bytes())?;
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
    let status = std::process::Command::new(&editor)
        .arg(file.path())
        .status()
        .with_context(|| format!("could not run {editor}"))?;
    if !status.success() {
        bail!("editor exited with an error");
    }
    std::fs::read_to_string(file.path()).context("could not read the edited alias")
}

fn config_command(args: ConfigArgs) -> Result<i32> {
    let store = Store::open()?;
    match args.command {
        ConfigCommand::Set(args) => {
            let project = project_scope(args.project)?;
            let value = config_value(args.value, args.stdin)?;
            store.set(&args.key, &value, args.secret, project.as_deref())?;
            ui::ok(&format!("Set {}", args.key));
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
            ui::ok(&format!("Unset {}", args.key));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_lists_every_subcommand() {
        let mut command = Cli::command();
        command.build();
        let text = ui::help_text(ui::Theme::plain(), 120);
        for sub in command.get_subcommands() {
            let name = sub.get_name();
            assert!(
                text.lines().any(|line| {
                    let cells: Vec<&str> = line.split_whitespace().collect();
                    cells.len() > 1 && cells[1].trim_end_matches(',') == name
                }),
                "{name}"
            );
        }
    }

    #[test]
    fn closing_lines_name_the_command_and_its_time() {
        let theme = ui::Theme::plain();
        let quick = Duration::from_millis(400);
        let long = Duration::from_millis(12_400);
        assert_eq!(closing_line(theme, "so compose ps", 0, quick, 100), None);
        assert_eq!(
            closing_line(theme, "so compose up -d", 0, long, 100).as_deref(),
            Some("✔ so compose up -d  12s")
        );
        assert_eq!(
            closing_line(theme, "so phpunit tests/Foo.php", 1, quick, 100).as_deref(),
            Some("✖ so phpunit tests/Foo.php exited 1  400ms")
        );
        let narrow = closing_line(theme, &format!("so {}", "x".repeat(200)), 2, long, 60).unwrap();
        assert!(ui::layout::width(&narrow) <= 60, "{narrow}");
        assert_eq!(
            invocation(["phpunit".to_string(), "a".to_string()].into_iter()),
            "so phpunit a"
        );
    }
}
