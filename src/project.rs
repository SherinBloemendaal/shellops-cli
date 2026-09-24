use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub root: PathBuf,
    pub app_env: String,
    pub dev: bool,
}

pub fn git_root() -> Result<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .context("git is required")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.trim();
        if detail.is_empty() {
            bail!("not inside a git repository");
        }
        bail!("not inside a git repository: {detail}");
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let path = PathBuf::from(text.trim());
    if path.as_os_str().is_empty() {
        bail!("not inside a git repository");
    }
    Ok(path)
}

pub fn discover() -> Result<Project> {
    let root = git_root()?;
    if !root.join("compose.yml").is_file() {
        bail!("{} has no compose.yml", root.display());
    }
    let dotenv = read_dotenv(&root.join(".env"));
    let app_env = std::env::var("APP_ENV")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| dotenv.get("APP_ENV").cloned())
        .unwrap_or_else(|| "prod".to_string());
    if let Some(value) = dotenv.get("APP_ENV")
        && std::env::var_os("APP_ENV").is_none()
    {
        // SAFETY: this process is single-threaded until commands spawn.
        unsafe { std::env::set_var("APP_ENV", value) };
    }
    load_dotenv_missing(&root.join(".env"));
    let dev = app_env == "dev";
    Ok(Project { root, app_env, dev })
}

pub fn read_dotenv(path: &Path) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let Ok(iter) = dotenvy::from_path_iter(path) else {
        return map;
    };
    for item in iter.flatten() {
        map.insert(item.0, item.1);
    }
    map
}

fn load_dotenv_missing(path: &Path) {
    if path.is_file() {
        let _ = dotenvy::from_path(path);
    }
}

pub fn version_value(root: &Path, dotenv: &BTreeMap<String, String>) -> String {
    if let Ok(value) = std::env::var("VERSION")
        && !value.is_empty()
    {
        return value;
    }
    if let Some(value) = dotenv.get("VERSION")
        && !value.is_empty()
    {
        return value.clone();
    }
    std::fs::read_to_string(root.join("VERSION"))
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| "latest".to_string())
}

pub fn home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .context("could not determine the home directory")
}

pub fn shellops_home() -> Result<PathBuf> {
    Ok(home_dir()?.join(".shellops"))
}
