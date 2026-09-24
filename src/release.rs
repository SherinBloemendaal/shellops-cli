use anyhow::{Context, Result, bail};
use chrono::Local;
use semver::Version;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::notes;
use crate::ui;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bump {
    Patch,
    Minor,
    Major,
    Current,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Stable,
    Rc,
    Beta,
    Alpha,
}

impl Channel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Rc => "rc",
            Self::Beta => "beta",
            Self::Alpha => "alpha",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub name: String,
    pub version: String,
    pub manifest: PathBuf,
    pub version_file: bool,
}

pub fn apply_bump(current: &str, bump: Bump) -> Result<String> {
    let mut version =
        Version::parse(current).with_context(|| format!("invalid version {current}"))?;
    version.pre = semver::Prerelease::EMPTY;
    version.build = semver::BuildMetadata::EMPTY;
    match bump {
        Bump::Patch => version.patch += 1,
        Bump::Minor => {
            version.minor += 1;
            version.patch = 0;
        }
        Bump::Major => {
            version.major += 1;
            version.minor = 0;
            version.patch = 0;
        }
        Bump::Current => {}
    }
    Ok(version.to_string())
}

pub fn next_prerelease_number(tags: &[String], package: &str, version: &str, channel: &str) -> u32 {
    let prefix = format!("{package}-v{version}-{channel}.");
    tags.iter()
        .filter_map(|tag| tag.strip_prefix(&prefix))
        .filter_map(|rest| rest.parse::<u32>().ok())
        .max()
        .unwrap_or(0)
        + 1
}

pub fn next_release_base(tags: &[String], today: &str) -> String {
    let prefix = format!("release-v{today}.");
    let max = tags
        .iter()
        .filter_map(|tag| tag.strip_prefix(&prefix))
        .map(|rest| rest.split('-').next().unwrap_or(rest))
        .filter_map(|seq| seq.parse::<u32>().ok())
        .max()
        .unwrap_or(0);
    format!("release-v{today}.{}", max + 1)
}

pub fn release_tag(base: &str, channel: Channel, number: u32) -> String {
    match channel {
        Channel::Stable => base.to_string(),
        Channel::Rc | Channel::Beta | Channel::Alpha => {
            format!("{base}-{}.{number}", channel.as_str())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ReleaseKey {
    year: u32,
    month: u32,
    day: u32,
    seq: u32,
}

pub fn previous_tag(tags: &[String], current: &str) -> Option<String> {
    let release_tags: Vec<&String> = tags
        .iter()
        .filter(|tag| tag.starts_with("release-v") && tag.as_str() != current)
        .collect();
    if let Some(tag) = release_tags.into_iter().max_by_key(|tag| release_key(tag)) {
        return Some(tag.clone());
    }
    tags.iter()
        .filter(|tag| {
            tag.starts_with('v') && !tag.starts_with("release-v") && tag.as_str() != current
        })
        .filter_map(|tag| {
            Version::parse(tag.trim_start_matches('v'))
                .ok()
                .map(|version| (version, tag))
        })
        .max_by(|left, right| left.0.cmp(&right.0))
        .map(|(_, tag)| tag.clone())
}

fn release_key(tag: &str) -> ReleaseKey {
    let rest = tag.trim_start_matches("release-v");
    let head = rest.split('-').next().unwrap_or(rest);
    let parts: Vec<u32> = head
        .split('.')
        .filter_map(|part| part.parse().ok())
        .collect();
    ReleaseKey {
        year: parts.first().copied().unwrap_or(0),
        month: parts.get(1).copied().unwrap_or(0),
        day: parts.get(2).copied().unwrap_or(0),
        seq: parts.get(3).copied().unwrap_or(0),
    }
}

pub fn run() -> Result<i32> {
    let root = crate::project::git_root()?;
    let branch = default_branch(&root)?;
    let current = git_stdout(&root, &["branch", "--show-current"])?;
    if current != branch {
        bail!("releases are cut from {branch} (currently on {current})");
    }
    if !git_stdout(&root, &["status", "--porcelain"])?.is_empty() {
        ui::warn("Git has uncommitted changes. They will not be part of the release commit.");
        if !ui::confirm_default("Continue?", true)? {
            return Ok(0);
        }
    }
    ui::info("Fetching origin");
    git_status(&root, &["fetch", "--tags", "--prune", "origin"])?;
    let head = git_stdout(&root, &["rev-parse", "HEAD"])?;
    let remote = git_stdout(&root, &["rev-parse", &format!("origin/{branch}")])?;
    if head != remote {
        bail!("HEAD does not match origin/{branch}");
    }
    let mut packages = select_packages(&root)?;
    if packages.is_empty() {
        return Ok(0);
    }
    let bump = select_bump(&packages)?;
    let channel = select_channel()?;
    for package in &mut packages {
        package.version = apply_bump(&package.version, bump)?;
    }
    let tags = git_lines(&root, &["tag", "-l"])?;
    let number = if channel == Channel::Stable {
        0
    } else {
        packages
            .iter()
            .map(|package| {
                next_prerelease_number(&tags, &package.name, &package.version, channel.as_str())
            })
            .max()
            .unwrap_or(1)
    };
    let today = Local::now().format("%Y.%m.%d").to_string();
    let base = next_release_base(&tags, &today);
    let tag = release_tag(&base, channel, number);
    let suffix = match channel {
        Channel::Stable => String::new(),
        _ => format!("-{}.{}", channel.as_str(), number),
    };
    for package in &packages {
        let marker = format!("{}-v{}{suffix}", package.name, package.version);
        if tags.iter().any(|existing| existing == &marker) {
            bail!("marker tag {marker} already exists");
        }
    }
    let manifest = packages
        .iter()
        .map(|package| format!("{}@{}", package.name, package.version))
        .collect::<Vec<_>>()
        .join(" ");
    ui::section("Release plan");
    ui::info(&format!("versions  {manifest}{suffix}"));
    ui::info(&format!("tag       {tag}"));
    ui::info(&format!("channel   {}", channel.as_str()));
    let subject = packages
        .iter()
        .map(|package| format!("{} {}", package.name, package.version))
        .collect::<Vec<_>>()
        .join(", ");
    if !ui::confirm_default(&format!("Commit, push {branch} and {tag}?"), true)? {
        ui::info("Aborted. Nothing changed.");
        return Ok(0);
    }
    for package in &packages {
        write_version(package)?;
    }
    git_status(&root, &["reset", "-q", "HEAD"])?;
    let mut add = vec!["add".to_string()];
    for package in &packages {
        add.push(package.manifest.to_string_lossy().to_string());
    }
    git_status(&root, &add.iter().map(String::as_str).collect::<Vec<_>>())?;
    let cached = git_stdout(&root, &["diff", "--cached", "--quiet"]);
    let committed = if cached.is_err() {
        let message = format!("chore(release): {subject}");
        git_status(&root, &["commit", "-m", &message])?;
        true
    } else {
        ui::info("No version changed; tagging current HEAD.");
        false
    };
    let message = format!(
        "{tag}\n\npackages: {manifest}\nchannel: {}\n",
        channel.as_str()
    );
    if let Err(err) = git_status(&root, &["tag", "-a", &tag, "-m", &message]) {
        if committed {
            let _ = git_status(&root, &["reset", "--mixed", "HEAD~1"]);
        }
        return Err(err);
    }
    ui::info(&format!("Pushing {branch} and {tag}"));
    if let Err(err) = git_status(
        &root,
        &[
            "push",
            "--atomic",
            "origin",
            &branch,
            &format!("refs/tags/{tag}"),
        ],
    ) {
        let _ = git_status(&root, &["tag", "-d", &tag]);
        if committed {
            let _ = git_status(&root, &["reset", "--mixed", "HEAD~1"]);
        }
        bail!("push failed. The local commit and tag were undone. {err}");
    }
    ui::ok(&format!("Pushed {tag}"));
    notes::offer(&root, &tag, channel)?;
    Ok(0)
}

fn select_packages(root: &Path) -> Result<Vec<Package>> {
    if let Some(packages) = workspace_packages(root)? {
        if packages.is_empty() {
            bail!("package.json workspaces did not match any packages");
        }
        let mut items: Vec<String> = packages
            .iter()
            .map(|package| format!("{} ({})", package.name, package.version))
            .collect();
        items.push("all".to_string());
        items.push("quit".to_string());
        let labels: Vec<&str> = items.iter().map(String::as_str).collect();
        let choice = ui::select("Packages", &labels)?;
        if choice == items.len() - 1 {
            return Ok(Vec::new());
        }
        if choice == items.len() - 2 {
            return Ok(packages);
        }
        return Ok(vec![packages[choice].clone()]);
    }
    let name = repo_name(root)?;
    let version_path = root.join("VERSION");
    let version = std::fs::read_to_string(&version_path)
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| "0.0.0".to_string());
    Ok(vec![Package {
        name,
        version,
        manifest: version_path,
        version_file: true,
    }])
}

fn select_bump(packages: &[Package]) -> Result<Bump> {
    for package in packages {
        let patch = apply_bump(&package.version, Bump::Patch).unwrap_or_default();
        let minor = apply_bump(&package.version, Bump::Minor).unwrap_or_default();
        let major = apply_bump(&package.version, Bump::Major).unwrap_or_default();
        ui::info(&format!(
            "{} {} -> patch {patch} / minor {minor} / major {major}",
            package.name, package.version
        ));
    }
    let choice = ui::select("Bump", &["patch", "minor", "major", "current", "quit"])?;
    match choice {
        0 => Ok(Bump::Patch),
        1 => Ok(Bump::Minor),
        2 => Ok(Bump::Major),
        3 => Ok(Bump::Current),
        _ => bail!("aborted"),
    }
}

fn select_channel() -> Result<Channel> {
    let choice = ui::select("Channel", &["stable", "rc", "beta", "alpha", "quit"])?;
    match choice {
        0 => Ok(Channel::Stable),
        1 => Ok(Channel::Rc),
        2 => Ok(Channel::Beta),
        3 => Ok(Channel::Alpha),
        _ => bail!("aborted"),
    }
}

fn workspace_packages(root: &Path) -> Result<Option<Vec<Package>>> {
    let path = root.join("package.json");
    if !path.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let json: Value = serde_json::from_str(&text)?;
    let Some(workspaces) = json.get("workspaces") else {
        return Ok(None);
    };
    let patterns = match workspaces {
        Value::Array(items) => items
            .iter()
            .filter_map(|item| item.as_str())
            .map(str::to_string)
            .collect::<Vec<_>>(),
        Value::Object(map) => map
            .get("packages")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    let mut packages = Vec::new();
    for pattern in patterns {
        for dir in expand_workspace(root, &pattern) {
            let manifest = dir.join("package.json");
            if !manifest.is_file() {
                continue;
            }
            let body = std::fs::read_to_string(&manifest)?;
            let value: Value = serde_json::from_str(&body)?;
            let name = dir
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("package")
                .to_string();
            let version = value
                .get("version")
                .and_then(|item| item.as_str())
                .unwrap_or("0.0.0")
                .to_string();
            packages.push(Package {
                name,
                version,
                manifest,
                version_file: false,
            });
        }
    }
    Ok(Some(packages))
}

fn expand_workspace(root: &Path, pattern: &str) -> Vec<PathBuf> {
    if let Some((dir, "*")) = pattern.rsplit_once('/')
        && !dir.contains('*')
    {
        let parent = root.join(dir);
        let Ok(entries) = std::fs::read_dir(&parent) else {
            return Vec::new();
        };
        let mut paths = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect::<Vec<_>>();
        paths.sort();
        return paths;
    }
    if pattern.contains('*') {
        return Vec::new();
    }
    vec![root.join(pattern)]
}

fn write_version(package: &Package) -> Result<()> {
    if package.version_file {
        std::fs::write(&package.manifest, format!("{}\n", package.version))
            .with_context(|| format!("could not write {}", package.manifest.display()))?;
        return Ok(());
    }
    let text = std::fs::read_to_string(&package.manifest)?;
    let mut json: Value = serde_json::from_str(&text)?;
    if let Some(object) = json.as_object_mut() {
        object.insert(
            "version".to_string(),
            Value::String(package.version.clone()),
        );
    }
    let body = serde_json::to_string_pretty(&json)? + "\n";
    std::fs::write(&package.manifest, body)?;
    Ok(())
}

fn repo_name(root: &Path) -> Result<String> {
    if let Ok(url) = git_stdout(root, &["remote", "get-url", "origin"])
        && let Some((_, repo)) = notes::parse_github_remote(url.trim())
    {
        return Ok(repo);
    }
    root.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .context("could not determine the repository name")
}

fn default_branch(root: &Path) -> Result<String> {
    let symbolic = git_stdout(
        root,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    );
    if let Ok(name) = symbolic {
        return Ok(name.trim_start_matches("origin/").to_string());
    }
    Ok("main".to_string())
}

pub fn git_lines(root: &Path, args: &[&str]) -> Result<Vec<String>> {
    let text = git_stdout(root, args)?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

fn git_stdout(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .context("could not run git")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git {} failed: {}", args.join(" "), stderr.trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn git_status(root: &Path, args: &[&str]) -> Result<()> {
    let status = Command::new("git")
        .args(args)
        .current_dir(root)
        .status()
        .context("could not run git")?;
    if status.success() {
        Ok(())
    } else {
        bail!("git {} failed", args.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bumps_and_prerelease_numbers() {
        assert_eq!(apply_bump("1.2.3", Bump::Patch).unwrap(), "1.2.4");
        assert_eq!(apply_bump("1.2.3", Bump::Minor).unwrap(), "1.3.0");
        assert_eq!(apply_bump("1.2.3", Bump::Major).unwrap(), "2.0.0");
        assert_eq!(apply_bump("1.2.3", Bump::Current).unwrap(), "1.2.3");
        let tags = vec![
            "auth-v1.0.34-rc.1".to_string(),
            "auth-v1.0.34-rc.4".to_string(),
            "auth-v1.0.35-rc.1".to_string(),
        ];
        assert_eq!(next_prerelease_number(&tags, "auth", "1.0.34", "rc"), 5);
        assert_eq!(next_prerelease_number(&tags, "auth", "1.0.36", "beta"), 1);
    }

    #[test]
    fn release_tags_use_the_next_sequence_and_channel() {
        let tags = vec![
            "release-v2026.09.24.1".to_string(),
            "release-v2026.09.24.2-rc.1".to_string(),
            "release-v2026.09.24.10".to_string(),
            "release-v2026.09.23.9".to_string(),
        ];
        let base = next_release_base(&tags, "2026.09.24");
        assert_eq!(base, "release-v2026.09.24.11");
        assert_eq!(release_tag(&base, Channel::Stable, 1), base);
        assert_eq!(
            release_tag(&base, Channel::Rc, 2),
            "release-v2026.09.24.11-rc.2"
        );
        assert_eq!(
            previous_tag(&tags, "release-v2026.09.24.11").as_deref(),
            Some("release-v2026.09.24.10")
        );
        let only_v = vec![
            "v1.0.0".to_string(),
            "v1.2.0".to_string(),
            "auth-v1.0.0".to_string(),
        ];
        assert_eq!(previous_tag(&only_v, "v9.0.0").as_deref(), Some("v1.2.0"));
    }
}
