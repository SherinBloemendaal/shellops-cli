use anyhow::{Result, bail};

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
    let labels: Vec<&str> = phases.iter().copied().map(phase_label).collect();
    let bars = ui::Bars::new(&labels);
    let project = project.clone();
    let results = std::thread::scope(|scope| {
        let mut joins = Vec::new();
        for (index, phase) in phases.into_iter().enumerate() {
            let project = project.clone();
            let bars = &bars;
            joins.push(scope.spawn(move || {
                let result = retry(|| run_phase(&project, phase, messenger));
                bars.finish(index, result.is_ok());
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
    bars.clear();
    for line in bars.summary(Theme::stdout()) {
        println!("{line}");
    }
    drop(bars);
    let failures: Vec<String> = labels
        .iter()
        .zip(&results)
        .filter_map(|(label, result)| {
            result
                .as_ref()
                .err()
                .map(|err| format!("{label}: {}", one_line(&err.to_string())))
        })
        .collect();
    if !failures.is_empty() {
        let phases = if failures.len() == 1 {
            "phase"
        } else {
            "phases"
        };
        return Err(ui::hinted(
            format!("{} cache {phases} failed", failures.len()),
            failures.join("\n"),
        ));
    }
    ui::ok("Caches cleared");
    Ok(0)
}

fn one_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
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

/// Runs one docker command of a phase with its output captured: three phases run in parallel
/// under the spinners, so streaming their output would tear the spinner lines apart. On
/// failure the last line the command printed becomes the error.
fn status_ok(project: &Project, args: &[String]) -> Result<()> {
    let output = compose::run_output(project, args)?;
    if output.status.success() {
        return Ok(());
    }
    let code = output.status.code().unwrap_or(1);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let command = describe(args);
    match last_line(&stderr).or_else(|| last_line(&stdout)) {
        Some(line) => bail!("{command} exited {code}: {line}"),
        None => bail!("{command} exited {code}"),
    }
}

/// A short name for a `docker compose exec` command line: `bin/console cache:clear`,
/// `redis-cli FLUSHALL`, `kill -USR2 1`.
fn describe(args: &[String]) -> String {
    if let Some(at) = args.iter().position(|arg| arg == "bin/console") {
        return args[at..]
            .iter()
            .take(2)
            .cloned()
            .collect::<Vec<_>>()
            .join(" ");
    }
    let mut rest = args
        .iter()
        .skip_while(|arg| *arg != "exec")
        .skip(1)
        .peekable();
    while rest.peek().is_some_and(|arg| arg.starts_with('-')) {
        let flag = rest.next();
        if flag.is_some_and(|flag| flag == "-u") {
            rest.next();
        }
    }
    rest.skip(1).take(3).cloned().collect::<Vec<_>>().join(" ")
}

fn last_line(text: &str) -> Option<String> {
    text.lines()
        .map(|line| ui::layout::strip_ansi(line).trim().to_string())
        .rfind(|line| !line.is_empty())
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
    fn failures_name_the_command_and_its_last_line() {
        assert_eq!(
            last_line("warming up\n\u{1b}[31m  [ERROR] cache dir not writable \u{1b}[0m\n\n"),
            Some("[ERROR] cache dir not writable".to_string())
        );
        assert_eq!(last_line("\n  \n"), None);
        let words = |text: &str| text.split(' ').map(String::from).collect::<Vec<_>>();
        assert_eq!(
            describe(&words(
                "compose exec -u www-data php php -d opcache.enable_cli=0 bin/console cache:clear --env=prod"
            )),
            "bin/console cache:clear"
        );
        assert_eq!(
            describe(&words("compose exec redis redis-cli FLUSHALL")),
            "redis-cli FLUSHALL"
        );
        assert_eq!(
            describe(&words("compose exec php kill -USR2 1")),
            "kill -USR2 1"
        );
        assert_eq!(one_line("a\n  b \n\nc"), "a b c");
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
