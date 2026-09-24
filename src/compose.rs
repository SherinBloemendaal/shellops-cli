use anyhow::{Context, Result, bail};
use std::process::{Command, Output};

use crate::build;
use crate::project::Project;
use crate::ui;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvBinding {
    pub key: String,
    pub value: String,
}

pub fn injected_env() -> Vec<EnvBinding> {
    let mut out = Vec::new();
    if env_missing("UID") {
        out.push(EnvBinding {
            key: "UID".to_string(),
            value: rustix::process::getuid().as_raw().to_string(),
        });
    }
    if env_missing("GID") {
        out.push(EnvBinding {
            key: "GID".to_string(),
            value: rustix::process::getgid().as_raw().to_string(),
        });
    }
    if env_missing("USER")
        && let Some(user) = current_user()
    {
        out.push(EnvBinding {
            key: "USER".to_string(),
            value: user,
        });
    }
    out
}

fn env_missing(key: &str) -> bool {
    std::env::var(key).ok().is_none_or(|value| value.is_empty())
}

fn current_user() -> Option<String> {
    if let Ok(user) = std::env::var("USER")
        && !user.is_empty()
    {
        return Some(user);
    }
    if let Ok(user) = std::env::var("USERNAME")
        && !user.is_empty()
    {
        return Some(user);
    }
    let output = Command::new("id").arg("-un").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!name.is_empty()).then_some(name)
}

pub fn apply_env(command: &mut Command) {
    for binding in injected_env() {
        command.env(&binding.key, &binding.value);
    }
}

pub fn run_status(project: &Project, args: &[String]) -> Result<i32> {
    let mut command = Command::new("docker");
    command.args(args).current_dir(&project.root);
    apply_env(&mut command);
    let status = command.status().context("could not run docker")?;
    Ok(status.code().unwrap_or(1))
}

pub fn run_output(project: &Project, args: &[String]) -> Result<Output> {
    let mut command = Command::new("docker");
    command.args(args).current_dir(&project.root);
    apply_env(&mut command);
    command.output().context("could not run docker")
}

pub fn compose(project: &Project, user_args: &[String]) -> Result<i32> {
    maybe_offer_build(project)?;
    let mut args = vec!["compose".to_string()];
    args.extend(user_args.iter().cloned());
    run_status(project, &args)
}

fn maybe_offer_build(project: &Project) -> Result<()> {
    if !project.dev || !ui::stdin_is_tty() {
        return Ok(());
    }
    let plan = build::plan(&project.root)?;
    if plan.services.is_empty() || plan.prefix.is_empty() {
        return Ok(());
    }
    let mut missing = Vec::new();
    for service in &plan.services {
        let image = format!("{}/{}:latest", plan.prefix, service.name);
        let inspect = run_output(
            project,
            &["image".to_string(), "inspect".to_string(), image.clone()],
        )?;
        if !inspect.status.success() {
            missing.push(image);
        }
    }
    if missing.is_empty() {
        return Ok(());
    }
    ui::warn(&format!("local image missing: {}", missing.join(", ")));
    if ui::confirm_default("Build local images now?", true)? {
        let code = build::build(project)?;
        if code != 0 {
            bail!("build failed");
        }
    } else {
        bail!("local image is missing");
    }
    Ok(())
}

pub fn services(project: &Project) -> Result<Vec<String>> {
    let output = run_output(
        project,
        &[
            "compose".to_string(),
            "config".to_string(),
            "--services".to_string(),
        ],
    )?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("docker compose config --services failed: {}", stderr.trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injection_skips_values_that_are_already_set() {
        let _guard = EnvLock::set("UID", "9");
        let keys: Vec<_> = injected_env().into_iter().map(|item| item.key).collect();
        assert!(!keys.iter().any(|key| key == "UID"));
    }

    struct EnvLock(&'static str, Option<std::ffi::OsString>);

    impl EnvLock {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var_os(key);
            unsafe { std::env::set_var(key, value) };
            Self(key, previous)
        }
    }

    impl Drop for EnvLock {
        fn drop(&mut self) {
            unsafe {
                match &self.1 {
                    Some(value) => std::env::set_var(self.0, value),
                    None => std::env::remove_var(self.0),
                }
            }
        }
    }
}
