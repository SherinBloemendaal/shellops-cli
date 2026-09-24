use anyhow::{Result, bail};
use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
use std::io::{self, IsTerminal};
use std::time::Duration;

use crate::compose::{self, services};
use crate::php::console_args;
use crate::project::Project;
use crate::ui::{self, Theme};

const ATTEMPTS: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Cache,
    Redis,
    Runtime,
}

pub fn phases_for(service_names: &[String]) -> Vec<Phase> {
    let mut phases = vec![Phase::Cache];
    if service_names.iter().any(|name| name == "redis") {
        phases.push(Phase::Redis);
    }
    phases.push(Phase::Runtime);
    phases
}

pub fn clear(project: &Project) -> Result<i32> {
    let names = services(project)?;
    let phases = phases_for(&names);
    let messenger = names.iter().any(|name| name == "messenger_workers");
    let theme = Theme::stderr();
    let multi = if io::stderr().is_terminal() {
        Some(MultiProgress::with_draw_target(ProgressDrawTarget::stderr()))
    } else {
        None
    };
    let bars: Vec<ProgressBar> = phases
        .iter()
        .map(|phase| {
            let bar = match &multi {
                Some(multi) => multi.add(ProgressBar::new_spinner()),
                None => ProgressBar::hidden(),
            };
            bar.set_style(style(theme));
            bar.set_message(phase_label(*phase));
            bar.enable_steady_tick(Duration::from_millis(80));
            bar
        })
        .collect();
    let project = project.clone();
    let results = std::thread::scope(|scope| {
        let mut joins = Vec::new();
        for (phase, bar) in phases.into_iter().zip(bars.iter()) {
            let project = project.clone();
            joins.push(scope.spawn(move || {
                let result = retry(|| run_phase(&project, phase, messenger));
                match &result {
                    Ok(()) => bar.finish_with_message(format!("{} done", phase_label(phase))),
                    Err(err) => {
                        bar.finish_with_message(format!("{} failed: {err}", phase_label(phase)))
                    }
                }
                result
            }));
        }
        joins
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|_| bail!("cache phase panicked"))
            })
            .collect::<Vec<_>>()
    });
    for result in results {
        result?;
    }
    ui::ok("cache clear finished");
    Ok(0)
}

fn style(theme: Theme) -> ProgressStyle {
    let template = if theme.color() {
        "{spinner:.cyan.bold} {msg}"
    } else {
        "{spinner} {msg}"
    };
    ProgressStyle::with_template(template)
        .unwrap_or_else(|_| ProgressStyle::default_spinner())
        .tick_chars(if theme.unicode() {
            "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ "
        } else {
            "|/-\\ "
        })
}

fn phase_label(phase: Phase) -> &'static str {
    match phase {
        Phase::Cache => "cache",
        Phase::Redis => "redis",
        Phase::Runtime => "runtime",
    }
}

fn retry(mut work: impl FnMut() -> Result<()>) -> Result<()> {
    let mut last = Ok(());
    for _ in 0..ATTEMPTS {
        match work() {
            Ok(()) => return Ok(()),
            Err(err) => last = Err(err),
        }
    }
    last
}

fn run_phase(project: &Project, phase: Phase, messenger: bool) -> Result<()> {
    match phase {
        Phase::Cache => cache_phase(project),
        Phase::Redis => status_ok(
            project,
            &[
                "compose".into(),
                "exec".into(),
                "redis".into(),
                "redis-cli".into(),
                "FLUSHALL".into(),
            ],
        ),
        Phase::Runtime => runtime_phase(project, messenger),
    }
}

fn cache_phase(project: &Project) -> Result<()> {
    status_ok(
        project,
        &[
            "compose".into(),
            "exec".into(),
            "php".into(),
            "rm".into(),
            "-Rf".into(),
            "var/cache".into(),
        ],
    )?;
    for command in [
        "doctrine:cache:clear-metadata",
        "doctrine:cache:clear-query",
        "doctrine:cache:clear-result",
    ] {
        let args = console_args(
            project.dev,
            &[command.into(), format!("--env={}", project.app_env)],
        );
        status_ok(project, &args)?;
    }
    let args = {
        let mut user = vec![
            "php".to_string(),
            "-d".to_string(),
            "opcache.enable_cli=0".to_string(),
            "-d".to_string(),
            "memory_limit=256M".to_string(),
            "bin/console".to_string(),
            "cache:clear".to_string(),
            format!("--env={}", project.app_env),
        ];
        let mut args = vec!["compose".to_string(), "exec".to_string()];
        if !project.dev {
            args.extend(["-u".to_string(), "www-data".to_string()]);
        }
        args.push("php".to_string());
        args.append(&mut user);
        args
    };
    status_ok(project, &args)
}

fn runtime_phase(project: &Project, messenger: bool) -> Result<()> {
    status_ok(
        project,
        &[
            "compose".into(),
            "exec".into(),
            "php".into(),
            "kill".into(),
            "-USR2".into(),
            "1".into(),
        ],
    )?;
    if messenger {
        let mut user = vec![
            "php".into(),
            "-d".into(),
            "opcache.enable_cli=0".into(),
            "-d".into(),
            "memory_limit=256M".into(),
            "bin/console".into(),
            "messenger:stop-worker".into(),
        ];
        let mut args = vec!["compose".into(), "exec".into()];
        if !project.dev {
            args.extend(["-u".into(), "www-data".into()]);
        }
        args.push("php".into());
        args.append(&mut user);
        status_ok(project, &args)?;
    }
    Ok(())
}

fn status_ok(project: &Project, args: &[String]) -> Result<()> {
    let code = compose::run_status(project, args)?;
    if code == 0 {
        Ok(())
    } else {
        bail!("command exited {code}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redis_and_messenger_add_their_phases() {
        let names = vec!["php".into(), "redis".into(), "messenger_workers".into()];
        assert_eq!(
            phases_for(&names),
            vec![Phase::Cache, Phase::Redis, Phase::Runtime]
        );
        assert_eq!(
            phases_for(&["php".into()]),
            vec![Phase::Cache, Phase::Runtime]
        );
    }

    #[test]
    fn retries_stop_after_three_failures() {
        let mut calls = 0;
        let err = retry(|| {
            calls += 1;
            bail!("nope");
        })
        .unwrap_err();
        assert_eq!(calls, 3);
        assert!(err.to_string().contains("nope"));
    }
}
