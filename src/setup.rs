use anyhow::{Context, Result, bail};
use comfy_table::{Attribute, Color};
use serde::Deserialize;
use std::io::{self, IsTerminal};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::config::Store;
use crate::identity;
use crate::ui::{self, Align, Sheet, Theme};

const HINT_INTERVAL_SECS: i64 = 24 * 60 * 60;
const STEPS: &[Step] = &[Step::Identity, Step::OpenRouter, Step::Github, Step::Docker];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Identity,
    OpenRouter,
    Github,
    Docker,
}

impl Step {
    pub fn parse(text: &str) -> Result<Self> {
        match text {
            "identity" => Ok(Self::Identity),
            "openrouter" => Ok(Self::OpenRouter),
            "github" => Ok(Self::Github),
            "docker" => Ok(Self::Docker),
            "status" => bail!("status is not a setup step"),
            other => bail!("unknown setup step {other}"),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::OpenRouter => "openrouter",
            Self::Github => "github",
            Self::Docker => "docker",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Identity => "Identity",
            Self::OpenRouter => "OpenRouter",
            Self::Github => "GitHub",
            Self::Docker => "Docker",
        }
    }

    fn key(self) -> String {
        format!("setup.{}", self.name())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Done,
    Later,
    Skipped,
    Missing,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Later => "later",
            Self::Skipped => "skipped",
            Self::Missing => "not set",
        }
    }

    fn cell(self, theme: Theme) -> comfy_table::Cell {
        match self {
            Self::Done => theme.cell(
                format!("{} {}", theme.icons().check, self.label()),
                Some(Color::Green),
                &[Attribute::Bold],
            ),
            Self::Later => theme.cell(
                format!("{} {}", theme.icons().hollow, self.label()),
                Some(Color::Yellow),
                &[],
            ),
            Self::Skipped | Self::Missing => theme.cell(
                format!("{} {}", theme.icons().hollow, self.label()),
                None,
                &[Attribute::Dim],
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Now,
    Later,
    Skip,
}

pub fn ensure_interactive(tty: bool) -> Result<()> {
    if tty {
        Ok(())
    } else {
        Err(ui::hinted(
            "so init needs a terminal.",
            "Set values with so config set, or run so init in a terminal.",
        ))
    }
}

pub fn status_of(store: &Store, step: Step) -> Status {
    match store.lookup(&step.key(), None) {
        Ok(Some(hit)) => match hit.value.as_str() {
            "done" => Status::Done,
            "later" => Status::Later,
            "skipped" => Status::Skipped,
            _ => Status::Missing,
        },
        _ => Status::Missing,
    }
}

pub fn set_status(store: &Store, step: Step, status: Status) -> Result<()> {
    let value = match status {
        Status::Done => "done",
        Status::Later => "later",
        Status::Skipped => "skipped",
        Status::Missing => {
            store.unset(&step.key(), None)?;
            return Ok(());
        }
    };
    store.set(&step.key(), value, false, None)
}

pub fn later_count(store: &Store) -> usize {
    STEPS
        .iter()
        .filter(|step| status_of(store, **step) == Status::Later)
        .count()
}

pub fn hint_message(later: usize) -> Option<String> {
    if later == 0 {
        None
    } else if later == 1 {
        Some("1 setup step is still open. Run so init.".to_string())
    } else {
        Some(format!("{later} setup steps are still open. Run so init."))
    }
}

pub fn hint_due(now: i64, last: Option<i64>) -> bool {
    !matches!(last, Some(checked) if now >= checked && now.saturating_sub(checked) < HINT_INTERVAL_SECS)
}

/// Prints the daily "setup steps are still open" reminder. Returns whether it printed.
pub fn notify_if_pending() -> bool {
    if std::env::var("SHELLOPS_NO_SETUP_HINT").ok().as_deref() == Some("1")
        || !io::stdout().is_terminal()
    {
        return false;
    }
    let Ok(store) = Store::open() else {
        return false;
    };
    let later = later_count(&store);
    let Some(message) = hint_message(later) else {
        return false;
    };
    let path = store.dir.join("setup-hint.json");
    let now = unix_now();
    let last = fs_json_i64(&path, "checked_at");
    if !hint_due(now, last) {
        return false;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, format!(r#"{{"checked_at":{now}}}"#));
    ui::notice(&message);
    true
}

pub fn run(step: Option<&str>, status: bool, check: bool) -> Result<i32> {
    if status || step == Some("status") {
        if step.is_some() && step != Some("status") {
            bail!("pass so init status --check");
        }
        print_status(check)?;
        return Ok(0);
    }
    if check {
        bail!("--check belongs to so init status");
    }
    ensure_interactive(io::stdin().is_terminal())?;
    let store = Store::open()?;
    let steps: Vec<Step> = match step {
        Some(name) => vec![Step::parse(name)?],
        None => STEPS.to_vec(),
    };
    let mut outcomes = Vec::new();
    for (index, step) in steps.iter().copied().enumerate() {
        if step_filter_is_all(steps.len()) && status_of(&store, step) == Status::Done {
            ui::ok(&format!("{} is already set up", step.title()));
            outcomes.push((step, Status::Done));
            continue;
        }
        ui::section(&format!(
            "{}  [{}/{}]",
            step.title(),
            index + 1,
            steps.len()
        ));
        let choice = ask_choice()?;
        let outcome = match choice {
            Choice::Later => {
                set_status(&store, step, Status::Later)?;
                ui::info("Left for later. Run so init when you want to finish it.");
                Status::Later
            }
            Choice::Skip => {
                set_status(&store, step, Status::Skipped)?;
                ui::info("Skipped.");
                Status::Skipped
            }
            Choice::Now => match run_step(&store, step) {
                Ok(()) => {
                    set_status(&store, step, Status::Done)?;
                    Status::Done
                }
                Err(err) => {
                    ui::report_error(&err);
                    match ask_failure()? {
                        Choice::Later => {
                            set_status(&store, step, Status::Later)?;
                            Status::Later
                        }
                        Choice::Skip => {
                            set_status(&store, step, Status::Skipped)?;
                            Status::Skipped
                        }
                        Choice::Now => {
                            run_step(&store, step)?;
                            set_status(&store, step, Status::Done)?;
                            Status::Done
                        }
                    }
                }
            },
        };
        outcomes.push((step, outcome));
    }
    print_outcomes(&outcomes);
    Ok(0)
}

fn step_filter_is_all(len: usize) -> bool {
    len == STEPS.len()
}

fn ask_choice() -> Result<Choice> {
    let index = ui::select("What do you want to do?", &["Set up now", "Later", "Skip"])?;
    Ok(match index {
        0 => Choice::Now,
        1 => Choice::Later,
        _ => Choice::Skip,
    })
}

fn ask_failure() -> Result<Choice> {
    let index = ui::select(
        "The check failed. Nothing was saved.",
        &["Try again", "Later", "Skip"],
    )?;
    Ok(match index {
        0 => Choice::Now,
        1 => Choice::Later,
        _ => Choice::Skip,
    })
}

fn run_step(store: &Store, step: Step) -> Result<()> {
    match step {
        Step::Identity => setup_identity(store),
        Step::OpenRouter => setup_openrouter(store),
        Step::Github => setup_github(store),
        Step::Docker => setup_docker(),
    }
}

fn setup_identity(store: &Store) -> Result<()> {
    let default = identity::gh_login();
    let login = ui::input("GitHub login", default.as_deref())?;
    identity::validate_login(&login)?;
    let spinner = ui::spinner("Reading SSH keys", false);
    let keys = identity::discover_keys()?;
    spinner.success(&format!(
        "Found {} SSH {}",
        keys.len(),
        plural(keys.len(), "key", "keys")
    ));
    let labels: Vec<String> = keys.iter().map(|key| key_label(key)).collect();
    let index = ui::select(
        "SSH key",
        &labels.iter().map(String::as_str).collect::<Vec<_>>(),
    )?;
    let spinner = ui::spinner(&format!("Checking github.com/{login}.keys"), false);
    let listed = identity::github_keys(store, &login, &identity::keys_base(), true)?;
    spinner.success(&format!("Read github.com/{login}.keys"));
    let identity = identity::accept_identity(&login, &keys[index], &listed)?;
    identity::save(store, &identity)?;
    ui::ok(&format!(
        "Saved identity {} {}",
        identity.github, identity.fingerprint
    ));
    Ok(())
}

/// `ed25519  me@laptop  SHA256:…`: the parts of a public key line a person recognizes,
/// instead of the base64 blob that would push the comment off the screen.
pub fn key_label(key: &str) -> String {
    let fingerprint = identity::fingerprint_of(key).unwrap_or_else(|_| "invalid key".into());
    let mut parts = key.split_whitespace();
    let kind = parts
        .next()
        .map(|kind| kind.trim_start_matches("ssh-"))
        .unwrap_or("key");
    let comment = parts.skip(1).collect::<Vec<_>>().join(" ");
    if comment.is_empty() {
        format!("{kind}  {fingerprint}")
    } else {
        format!("{kind}  {comment}  {fingerprint}")
    }
}

fn setup_openrouter(store: &Store) -> Result<()> {
    let key = ui::password("OpenRouter API key")?;
    let base = openrouter_base();
    let spinner = ui::spinner("Checking the OpenRouter key", false);
    let info = fetch_openrouter_key(&base, &key)?;
    spinner.success("OpenRouter accepted the key");
    println!(
        "{}",
        ui::panel(
            Theme::stdout(),
            vec![
                ("label", Theme::stdout().cell(&info.label, None, &[])),
                ("limit", Theme::stdout().cell(&info.limit, None, &[])),
                ("usage", Theme::stdout().cell(&info.usage, None, &[])),
            ],
        )
    );
    let spinner = ui::spinner("Loading models", false);
    let models = fetch_models(&base, &key)?;
    if models.is_empty() {
        bail!("OpenRouter returned no models");
    }
    spinner.success(&format!("Loaded {} models", models.len()));
    let current = store
        .lookup("openrouter.model", None)?
        .map(|hit| hit.value)
        .or_else(|| crate::config::inferred_default("openrouter.model"));
    let default = current
        .as_ref()
        .and_then(|model| models.iter().position(|item| item == model))
        .unwrap_or(0);
    let index = ui::fuzzy("Model", &models, default)?;
    let model = &models[index];
    if !ui::confirm_default(
        "Send a 5-token test prompt? This costs a fraction of a cent.",
        true,
    )? {
        bail!("OpenRouter test was skipped, so nothing was saved");
    }
    let spinner = ui::spinner("Sending a test prompt", false);
    let reply = probe_model(&base, &key, model)?;
    spinner.success(&format!("{model} replied: {}", one_line(&reply)));
    store.set("openrouter", &key, true, None)?;
    store.set("openrouter.model", model, false, None)?;
    ui::ok("Saved the OpenRouter key and model");
    Ok(())
}

fn setup_github(store: &Store) -> Result<()> {
    let gh_ok = Command::new("gh")
        .args(["auth", "token"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    let choice = if gh_ok {
        ui::select(
            "GitHub credentials",
            &["Use gh auth token", "Paste a token"],
        )?
    } else {
        1
    };
    if choice == 0 {
        let spinner = ui::spinner("Checking gh", false);
        let login = gh_api_login()?;
        store.unset("github", None)?;
        store.set("github.auth", "gh", false, None)?;
        spinner.success(&format!("GitHub via gh ({login})"));
        return Ok(());
    }
    let token = ui::password("GitHub token")?;
    let spinner = ui::spinner("Checking the GitHub token", false);
    let account = fetch_github_user(&github_api_base(), &token)?;
    drop(spinner);
    if !account.can_release {
        ui::warn(
            "The token lacks the repo scope or contents=write; so release may fail to publish.",
        );
    } else {
        ui::ok(&format!("Token can publish releases ({})", account.login));
    }
    store.unset("github.auth", None)?;
    store.set("github", &token, true, None)?;
    ui::ok(&format!("Saved a GitHub token for {}", account.login));
    Ok(())
}

fn setup_docker() -> Result<()> {
    let theme = Theme::stdout();
    let info = probe_command("docker", &["info"]);
    let compose = probe_command("docker", &["compose", "version"]);
    println!(
        "{}",
        ui::panel(
            theme,
            vec![
                ("daemon", mark(theme, info)),
                ("compose", mark(theme, compose)),
            ],
        )
    );
    if !(info && compose) {
        bail!("Docker is not ready");
    }
    ui::ok("Docker is ready");
    Ok(())
}

fn mark(theme: Theme, ok: bool) -> comfy_table::Cell {
    if ok {
        theme.cell("ready", Some(Color::Green), &[Attribute::Bold])
    } else {
        theme.cell("not ready", Some(Color::Red), &[Attribute::Bold])
    }
}

pub fn probe_command(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub fn print_status(check: bool) -> Result<i32> {
    let store = Store::open()?;
    let theme = Theme::stdout();
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
    let spinner = ui::spinner("Checking the finished steps", !check);
    let rows: Vec<(Step, Status, String)> = STEPS
        .iter()
        .map(|step| {
            let status = status_of(&store, *step);
            let detail = if check && status == Status::Done {
                check_step(&store, *step)
            } else {
                quiet_detail(&store, *step, status)
            };
            (*step, status, detail)
        })
        .collect();
    drop(spinner);
    for (step, status, detail) in rows {
        sheet.row(vec![
            theme.cell(step.title(), Some(Color::Cyan), &[Attribute::Bold]),
            status.cell(theme),
            theme.cell(detail, None, &[]),
        ]);
    }
    println!("{sheet}");
    if STEPS
        .iter()
        .any(|step| status_of(&store, *step) != Status::Done)
    {
        ui::hint("Run so init to finish the open steps.");
    }
    Ok(0)
}

fn print_outcomes(outcomes: &[(Step, Status)]) {
    let theme = Theme::stdout();
    ui::section("Setup");
    let mut sheet = Sheet::new(theme, &[("step", Align::Left), ("status", Align::Left)]);
    for (step, status) in outcomes {
        sheet.row(vec![
            theme.cell(step.title(), Some(Color::Cyan), &[Attribute::Bold]),
            status.cell(theme),
        ]);
    }
    println!("{sheet}");
}

fn quiet_detail(store: &Store, step: Step, status: Status) -> String {
    if status != Status::Done {
        return "-".to_string();
    }
    match step {
        Step::Identity => identity::load(store)
            .ok()
            .flatten()
            .map(|identity| format!("{} {}", identity.github, identity.fingerprint))
            .unwrap_or_else(|| "saved".to_string()),
        Step::OpenRouter => store
            .lookup("openrouter.model", None)
            .ok()
            .flatten()
            .map(|hit| hit.value)
            .unwrap_or_else(|| "key saved".to_string()),
        Step::Github => match store.lookup("github.auth", None) {
            Ok(Some(hit)) if hit.value == "gh" => "via gh".to_string(),
            _ => "token saved".to_string(),
        },
        Step::Docker => "checked".to_string(),
    }
}

fn check_step(store: &Store, step: Step) -> String {
    match step {
        Step::Identity => match identity::load(store).ok().flatten() {
            Some(identity) => {
                match identity::key_status(
                    store,
                    &identity.github,
                    &identity.public_key,
                    &identity::keys_base(),
                ) {
                    identity::KeyCheck::Listed => format!("{} key listed", identity.github),
                    identity::KeyCheck::Missing => {
                        format!("{} key missing on GitHub", identity.github)
                    }
                    identity::KeyCheck::Unknown => "could not check GitHub".to_string(),
                }
            }
            None => "identity missing".to_string(),
        },
        Step::OpenRouter => match store.lookup("openrouter", None) {
            Ok(Some(hit)) => match fetch_openrouter_key(&openrouter_base(), &hit.value) {
                Ok(info) => format!("key ok ({})", info.label),
                Err(err) => err.to_string(),
            },
            _ => "key missing".to_string(),
        },
        Step::Github => check_github(store),
        Step::Docker => {
            if probe_command("docker", &["info"])
                && probe_command("docker", &["compose", "version"])
            {
                "ready".to_string()
            } else {
                "not ready".to_string()
            }
        }
    }
}

fn check_github(store: &Store) -> String {
    if store
        .lookup("github.auth", None)
        .ok()
        .flatten()
        .is_some_and(|hit| hit.value == "gh")
    {
        return match gh_api_login() {
            Ok(login) => format!("gh ({login})"),
            Err(err) => err.to_string(),
        };
    }
    match store.lookup("github", None) {
        Ok(Some(hit)) => match fetch_github_user(&github_api_base(), &hit.value) {
            Ok(account) => format!(
                "{} ({})",
                account.login,
                if account.can_release {
                    "can release"
                } else {
                    "limited scopes"
                }
            ),
            Err(err) => err.to_string(),
        },
        _ => "token missing".to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyInfo {
    pub label: String,
    pub limit: String,
    pub usage: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubAccount {
    pub login: String,
    pub can_release: bool,
}

pub fn fetch_openrouter_key(base: &str, key: &str) -> Result<KeyInfo> {
    let url = format!("{}/api/v1/key", base.trim_end_matches('/'));
    let text = get_bearer(&url, key)?;
    let json: serde_json::Value =
        serde_json::from_str(&text).context("could not parse OpenRouter")?;
    let data = json.get("data").unwrap_or(&json);
    Ok(KeyInfo {
        label: json_string(data, "label").unwrap_or_else(|| "key".to_string()),
        limit: json_number(data, "limit"),
        usage: json_number(data, "usage"),
    })
}

pub fn fetch_models(base: &str, key: &str) -> Result<Vec<String>> {
    let url = format!("{}/api/v1/models", base.trim_end_matches('/'));
    let text = get_bearer(&url, key)?;
    let parsed: Models =
        serde_json::from_str(&text).context("could not parse OpenRouter models")?;
    let mut ids: Vec<String> = parsed.data.into_iter().map(|model| model.id).collect();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

pub fn probe_model(base: &str, key: &str, model: &str) -> Result<String> {
    let url = format!("{}/api/v1/chat/completions", base.trim_end_matches('/'));
    let body = serde_json::json!({
        "model": model,
        "max_tokens": 5,
        "messages": [{"role": "user", "content": "Reply with ok."}]
    });
    let text = post_bearer(&url, key, body)?;
    let json: serde_json::Value =
        serde_json::from_str(&text).context("could not parse the test reply")?;
    json.pointer("/choices/0/message/content")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .context("OpenRouter test reply was empty")
}

pub fn fetch_github_user(base: &str, token: &str) -> Result<GithubAccount> {
    let url = format!("{}/user", base.trim_end_matches('/'));
    let (text, headers) = get_bearer_headers(&url, token)?;
    let json: serde_json::Value = serde_json::from_str(&text).context("could not parse GitHub")?;
    let login = json
        .get("login")
        .and_then(|value| value.as_str())
        .context("GitHub user has no login")?
        .to_string();
    let scopes = header_value(&headers, "x-oauth-scopes");
    let permissions = header_value(&headers, "x-accepted-github-permissions");
    Ok(GithubAccount {
        login,
        can_release: release_scopes_ok(&scopes, &permissions),
    })
}

pub fn release_scopes_ok(oauth_scopes: &str, accepted_permissions: &str) -> bool {
    let oauth = oauth_scopes.split(',').any(|scope| {
        let scope = scope.trim();
        scope == "repo" || scope == "public_repo"
    });
    let fine = accepted_permissions.split([',', ';']).any(|item| {
        let item = item.split_whitespace().collect::<String>();
        item == "contents=write" || item.starts_with("contents=write")
    });
    oauth || fine
}

pub fn openrouter_base() -> String {
    std::env::var("SHELLOPS_OPENROUTER_API").unwrap_or_else(|_| "https://openrouter.ai".to_string())
}

pub fn github_api_base() -> String {
    std::env::var("SHELLOPS_GITHUB_API").unwrap_or_else(|_| "https://api.github.com".to_string())
}

fn gh_api_login() -> Result<String> {
    let output = Command::new("gh")
        .args(["api", "user", "--jq", ".login"])
        .output()
        .context("could not run gh")?;
    if !output.status.success() {
        bail!("gh is not logged in");
    }
    let login = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if login.is_empty() {
        bail!("gh did not return a login");
    }
    Ok(login)
}

#[derive(Debug, Deserialize)]
struct Models {
    data: Vec<Model>,
}

#[derive(Debug, Deserialize)]
struct Model {
    id: String,
}

fn get_bearer(url: &str, token: &str) -> Result<String> {
    Ok(get_bearer_headers(url, token)?.0)
}

fn get_bearer_headers(url: &str, token: &str) -> Result<(String, Vec<(String, String)>)> {
    let response = agent()
        .get(url)
        .set("Authorization", &format!("Bearer {token}"))
        .set("Accept", "application/vnd.github+json")
        .set("User-Agent", concat!("so/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|err| anyhow::anyhow!("request failed: {err}"))?;
    let headers = response
        .headers_names()
        .into_iter()
        .filter_map(|name| {
            response
                .header(&name)
                .map(|value| (name.clone(), value.to_string()))
        })
        .collect();
    let text = response
        .into_string()
        .context("could not read the response")?;
    Ok((text, headers))
}

fn post_bearer(url: &str, token: &str, body: serde_json::Value) -> Result<String> {
    let response = agent()
        .post(url)
        .set("Authorization", &format!("Bearer {token}"))
        .set("Content-Type", "application/json")
        .set("User-Agent", concat!("so/", env!("CARGO_PKG_VERSION")))
        .send_json(body)
        .map_err(|err| anyhow::anyhow!("request failed: {err}"))?;
    response
        .into_string()
        .context("could not read the response")
}

fn header_value(headers: &[(String, String)], name: &str) -> String {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.clone())
        .unwrap_or_default()
}

fn json_string(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|item| item.as_str())
        .map(str::to_string)
}

fn json_number(value: &serde_json::Value, key: &str) -> String {
    match value.get(key) {
        Some(serde_json::Value::Null) | None => "none".to_string(),
        Some(other) => other.to_string().trim_matches('"').to_string(),
    }
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(30))
        .redirects(8)
        .build()
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn plural(count: usize, one: &str, many: &str) -> &'static str {
    if count == 1 { leak(one) } else { leak(many) }
}

fn leak(text: &str) -> &'static str {
    match text {
        "key" => "key",
        "keys" => "keys",
        _ => "items",
    }
}

fn fs_json_i64(path: &Path, key: &str) -> Option<i64> {
    let text = std::fs::read_to_string(path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    json.get(key).and_then(|value| value.as_i64())
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn later_is_hinted_once_per_day_and_a_terminal_is_required() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::at(dir.path());
        assert!(ensure_interactive(false).is_err());
        set_status(&store, Step::OpenRouter, Status::Later).unwrap();
        set_status(&store, Step::Github, Status::Skipped).unwrap();
        assert_eq!(later_count(&store), 1);
        assert_eq!(
            hint_message(1).as_deref(),
            Some("1 setup step is still open. Run so init.")
        );
        assert!(hint_due(1_000, None));
        assert!(!hint_due(1_000 + 60, Some(1_000)));
        assert!(hint_due(1_000 + HINT_INTERVAL_SECS, Some(1_000)));
        assert!(release_scopes_ok("repo, read:org", ""));
        assert!(release_scopes_ok("", "contents=write"));
        assert!(!release_scopes_ok("read:org", ""));
    }

    #[test]
    fn a_failed_openrouter_check_saves_nothing() {
        let server = serve(401, r#"{"error":"no"}"#);
        let err = fetch_openrouter_key(&server, "secret").unwrap_err();
        assert!(err.to_string().contains("request failed"));
        let dir = tempfile::tempdir().unwrap();
        let store = Store::at(dir.path());
        assert!(store.lookup("openrouter", None).unwrap().is_none());
    }

    #[test]
    fn openrouter_and_github_checks_parse_a_local_server() {
        let body = r#"{"data":{"label":"dev","limit":null,"usage":1.5},"id":"openai/gpt-4o-mini"}"#;
        let server = serve(200, body);
        let info = fetch_openrouter_key(&server, "secret").unwrap();
        assert_eq!(info.label, "dev");
        assert_eq!(info.limit, "none");
        let models_server = serve(
            200,
            r#"{"data":[{"id":"openai/gpt-4o-mini"},{"id":"a/b"}]}"#,
        );
        let models = fetch_models(&models_server, "secret").unwrap();
        assert_eq!(
            models,
            vec!["a/b".to_string(), "openai/gpt-4o-mini".to_string()]
        );
        let github = serve_headers(
            200,
            r#"{"login":"octocat"}"#,
            &[("x-oauth-scopes", "repo, gist")],
        );
        let account = fetch_github_user(&github, "token").unwrap();
        assert_eq!(account.login, "octocat");
        assert!(account.can_release);
    }

    fn serve(status: u16, body: &str) -> String {
        serve_headers(status, body, &[])
    }

    fn serve_headers(status: u16, body: &str, extra: &[(&str, &str)]) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let body = body.to_string();
        let extra: Vec<(String, String)> = extra
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect();
        std::thread::spawn(move || {
            for _ in 0..4 {
                let Ok((mut socket, _)) = listener.accept() else {
                    break;
                };
                let _ = socket.set_read_timeout(Some(Duration::from_secs(2)));
                let mut buf = [0_u8; 8192];
                let _ = std::io::Read::read(&mut socket, &mut buf);
                let mut headers = format!(
                    "HTTP/1.1 {status} OK\r\nContent-Length: {}\r\nConnection: close\r\n",
                    body.len()
                );
                for (key, value) in &extra {
                    headers.push_str(&format!("{key}: {value}\r\n"));
                }
                headers.push_str("\r\n");
                let _ = std::io::Write::write_all(&mut socket, headers.as_bytes());
                let _ = std::io::Write::write_all(&mut socket, body.as_bytes());
            }
        });
        format!("http://127.0.0.1:{port}")
    }
}
