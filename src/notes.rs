use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use crate::config::{self, Store, Visibility};
use crate::release::{Channel, previous_tag};
use crate::ui;

pub const DIFF_CAP: usize = 80_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseFlags {
    pub prerelease: bool,
    pub make_latest: bool,
}

pub fn flags_for(channel: Channel) -> ReleaseFlags {
    match channel {
        Channel::Stable => ReleaseFlags {
            prerelease: false,
            make_latest: true,
        },
        Channel::Rc | Channel::Beta | Channel::Alpha => ReleaseFlags {
            prerelease: true,
            make_latest: false,
        },
    }
}

pub fn diff_path_allowed(path: &str) -> bool {
    let banned = [
        "Cargo.lock",
        "package-lock.json",
        "yarn.lock",
        "composer.lock",
        "pnpm-lock.yaml",
    ];
    if banned.iter().any(|name| path.ends_with(name)) {
        return false;
    }
    let markers = ["/vendor/", "/node_modules/", "/target/", "/dist/"];
    if markers.iter().any(|marker| path.contains(marker)) {
        return false;
    }
    !path.starts_with("vendor/")
        && !path.starts_with("node_modules/")
        && !path.starts_with("target/")
        && !path.starts_with("dist/")
}

pub fn is_migration(path: &str) -> bool {
    path == "migrations" || path.starts_with("migrations/") || path.contains("/migrations/")
}

pub fn is_env_example(path: &str) -> bool {
    Path::new(path).file_name().and_then(|name| name.to_str()) == Some(".env.example")
}

pub fn cap_text(text: &str, cap: usize) -> String {
    if text.len() <= cap {
        return text.to_string();
    }
    let mut end = cap;
    while !text.is_char_boundary(end) && end > 0 {
        end -= 1;
    }
    format!("{}\n\n[diff truncated]\n", &text[..end])
}

pub fn build_prompt(
    log: &str,
    stat: &str,
    diff: &str,
    migrations: &str,
    env_example: &str,
) -> String {
    format!(
        "Write technical developer release notes in Markdown.\n\
         Include: a short summary, changes grouped by type, breaking changes, migrations, and env/config changes.\n\
         Use only the evidence below. Do not invent changes.\n\n\
         ## git log\n{log}\n\n\
         ## diff stat\n{stat}\n\n\
         ## migrations signal\n{migrations}\n\n\
         ## env example signal\n{env_example}\n\n\
         ## filtered diff\n{diff}\n"
    )
}

pub fn offer(root: &Path, tag: &str, channel: Channel) -> Result<()> {
    if !ui::stdin_is_tty() {
        return Ok(());
    }
    if !ui::confirm_default("Create a GitHub Release?", false)? {
        return Ok(());
    }
    let store = Store::open()?;
    let mut notes = generate(root, tag, &store)?;
    loop {
        println!("{notes}");
        match ui::select(
            "GitHub Release",
            &["Publish", "Edit in $EDITOR", "Regenerate", "Cancel"],
        )? {
            0 => {
                publish(root, tag, &notes, channel, &store)?;
                ui::ok("GitHub Release created");
                return Ok(());
            }
            1 => notes = edit_notes(&notes)?,
            2 => notes = generate(root, tag, &store)?,
            _ => {
                ui::info("GitHub Release cancelled");
                return Ok(());
            }
        }
    }
}

fn generate(root: &Path, tag: &str, store: &Store) -> Result<String> {
    let tags = crate::release::git_lines(root, &["tag", "-l"])?;
    let base = previous_tag(&tags, tag);
    let range = match &base {
        Some(base) => format!("{base}..{tag}"),
        None => tag.to_string(),
    };
    let log = git_text(
        root,
        &["log", "--no-merges", "--pretty=format:%h %s", &range],
    )?;
    let stat = git_text(root, &["diff", "--stat", &range])?;
    let names = git_text(root, &["diff", "--name-only", &range])?;
    let paths: Vec<&str> = names
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && diff_path_allowed(line))
        .collect();
    let migration_paths: Vec<&str> = paths
        .iter()
        .copied()
        .filter(|path| is_migration(path))
        .collect();
    let env_paths: Vec<&str> = paths
        .iter()
        .copied()
        .filter(|path| is_env_example(path))
        .collect();
    let migrations = section_diff(root, &range, &migration_paths)?;
    let env_example = section_diff(root, &range, &env_paths)?;
    let filtered_paths: Vec<&str> = paths
        .iter()
        .copied()
        .filter(|path| !is_migration(path) && !is_env_example(path))
        .collect();
    let diff = cap_text(&section_diff(root, &range, &filtered_paths)?, DIFF_CAP);
    let prompt = build_prompt(&log, &stat, &diff, &migrations, &env_example);
    let key = secret_or_plain(store, "openrouter", root)?;
    let model = store
        .lookup("openrouter.model", Some(root))?
        .map(|hit| hit.value)
        .or_else(|| config::inferred_default("openrouter.model"))
        .context("set openrouter.model with so config set openrouter.model <model>")?;
    complete_openrouter(&key, &model, &prompt)
}

fn section_diff(root: &Path, range: &str, paths: &[&str]) -> Result<String> {
    if paths.is_empty() {
        return Ok("none".to_string());
    }
    let mut args = vec!["diff", range, "--"];
    args.extend(paths);
    git_text(root, &args)
}

fn secret_or_plain(store: &Store, key: &str, root: &Path) -> Result<String> {
    let hit = store.lookup(key, Some(root))?.with_context(|| {
        format!("set the {key} secret with so config set {key} --secret --stdin")
    })?;
    if hit.visibility != Visibility::Secret && hit.scope == crate::config::Scope::Inferred {
        bail!("{key} is not set");
    }
    Ok(hit.value)
}

fn complete_openrouter(key: &str, model: &str, prompt: &str) -> Result<String> {
    let body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}]
    });
    let response = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(120))
        .build()
        .post("https://openrouter.ai/api/v1/chat/completions")
        .set("Authorization", &format!("Bearer {key}"))
        .set("Content-Type", "application/json")
        .send_json(body)
        .map_err(|err| anyhow::anyhow!("OpenRouter request failed: {err}"))?;
    let text = response
        .into_string()
        .context("could not read OpenRouter")?;
    let json: serde_json::Value =
        serde_json::from_str(&text).context("could not parse OpenRouter")?;
    json.pointer("/choices/0/message/content")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .context("OpenRouter returned an empty note")
}

fn edit_notes(current: &str) -> Result<String> {
    let mut file = tempfile::NamedTempFile::new().context("could not create a notes file")?;
    std::io::Write::write_all(file.as_file_mut(), current.as_bytes())?;
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
    let status = Command::new(&editor)
        .arg(file.path())
        .status()
        .with_context(|| format!("could not run {editor}"))?;
    if !status.success() {
        bail!("editor exited with an error");
    }
    std::fs::read_to_string(file.path()).context("could not read the edited notes")
}

fn publish(root: &Path, tag: &str, notes: &str, channel: Channel, store: &Store) -> Result<()> {
    let (owner, repo) = github_repo(root)?;
    let token = match store.lookup("github", Some(root))? {
        Some(hit) => hit.value,
        None => gh_token()?,
    };
    let flags = flags_for(channel);
    let body = serde_json::json!({
        "tag_name": tag,
        "name": tag,
        "body": notes,
        "prerelease": flags.prerelease,
        "make_latest": if flags.make_latest { "true" } else { "false" },
    });
    let url = format!("https://api.github.com/repos/{owner}/{repo}/releases");
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(60))
        .build()
        .post(&url)
        .set("Authorization", &format!("Bearer {token}"))
        .set("Accept", "application/vnd.github+json")
        .set("User-Agent", concat!("so/", env!("CARGO_PKG_VERSION")))
        .send_json(body)
        .map_err(|err| anyhow::anyhow!("GitHub Release request failed: {err}"))?;
    Ok(())
}

pub fn github_repo(root: &Path) -> Result<(String, String)> {
    let url = git_text(root, &["remote", "get-url", "origin"])?;
    parse_github_remote(url.trim()).context("origin is not a GitHub repository")
}

pub fn parse_github_remote(url: &str) -> Option<(String, String)> {
    let rest = url
        .strip_prefix("git@github.com:")
        .or_else(|| url.strip_prefix("https://github.com/"))
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))?;
    let rest = rest.trim_end_matches('/').trim_end_matches(".git");
    let (owner, repo) = rest.split_once('/')?;
    if owner.is_empty() || repo.is_empty() || repo.contains('/') {
        return None;
    }
    Some((owner.to_string(), repo.to_string()))
}

fn gh_token() -> Result<String> {
    let output = Command::new("gh")
        .args(["auth", "token"])
        .output()
        .context("could not run gh auth token")?;
    if !output.status.success() {
        bail!("set a github secret or run gh auth login");
    }
    let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if token.is_empty() {
        bail!("gh auth token returned an empty token");
    }
    Ok(token)
}

fn git_text(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .context("could not run git")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git {} failed: {}", args.join(" "), stderr.trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_is_latest_and_prerelease_is_not() {
        assert!(!flags_for(Channel::Stable).prerelease);
        assert!(flags_for(Channel::Stable).make_latest);
        assert!(flags_for(Channel::Rc).prerelease);
        assert!(!flags_for(Channel::Beta).make_latest);
        assert!(flags_for(Channel::Alpha).prerelease);
    }

    #[test]
    fn signals_and_cap() {
        assert!(is_migration("migrations/Version1.php"));
        assert!(is_env_example("app/.env.example"));
        assert!(!diff_path_allowed("app/vendor/autoload.php"));
        assert!(diff_path_allowed("src/Kernel.php"));
        let capped = cap_text(&"a".repeat(20), 8);
        assert!(capped.contains("[diff truncated]"));
        assert!(capped.len() < 40);
    }

    #[test]
    fn github_remote_parses_ssh_and_https() {
        assert_eq!(
            parse_github_remote("git@github.com:SherinBloemendaal/shellops-cli.git").unwrap(),
            ("SherinBloemendaal".into(), "shellops-cli".into())
        );
        assert_eq!(
            parse_github_remote("https://github.com/SherinBloemendaal/auth").unwrap(),
            ("SherinBloemendaal".into(), "auth".into())
        );
    }
}
