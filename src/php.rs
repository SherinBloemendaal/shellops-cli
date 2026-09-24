use anyhow::{Context, Result, bail};
use std::path::Path;

use crate::compose::{run_output, run_status};
use crate::project::Project;

pub fn console_args(dev: bool, user: &[String]) -> Vec<String> {
    let mut args = vec!["compose".to_string(), "exec".to_string()];
    if !dev {
        args.extend(["-u".to_string(), "www-data".to_string()]);
    }
    args.extend([
        "php".to_string(),
        "php".to_string(),
        "-d".to_string(),
        "opcache.enable_cli=0".to_string(),
        "-d".to_string(),
        "memory_limit=8G".to_string(),
        "bin/console".to_string(),
    ]);
    args.extend(user.iter().cloned());
    args
}

pub fn composer_args(user: &[String]) -> Vec<String> {
    let mut args = vec![
        "compose".to_string(),
        "run".to_string(),
        "--rm".to_string(),
        "php".to_string(),
        "php".to_string(),
        "-d".to_string(),
        "opcache.enable_cli=0".to_string(),
        "/usr/local/bin/composer".to_string(),
        "--profile".to_string(),
    ];
    if user
        .first()
        .is_some_and(|arg| arg == "install" || arg == "i")
    {
        args.push("--optimize-autoloader".to_string());
        args.push("--classmap-authoritative".to_string());
    }
    args.extend(user.iter().cloned());
    args
}

pub fn exec_php(defines: &[&str], command: &[String]) -> Vec<String> {
    let mut args = vec![
        "compose".to_string(),
        "exec".to_string(),
        "php".to_string(),
        "php".to_string(),
    ];
    for define in defines {
        args.push("-d".to_string());
        args.push((*define).to_string());
    }
    args.extend(command.iter().cloned());
    args
}

pub fn yarn_args(user: &[String]) -> Vec<String> {
    let mut args = vec![
        "compose".to_string(),
        "exec".to_string(),
        "vue".to_string(),
        "yarn".to_string(),
    ];
    args.extend(user.iter().cloned());
    args
}

pub fn csf_args() -> Vec<String> {
    exec_php(
        &["opcache.enable_cli=0"],
        &[
            "tools/php-cs-fixer/vendor/bin/php-cs-fixer".into(),
            "fix".into(),
        ],
    )
}

pub fn phpstan_args(user: &[String]) -> Vec<String> {
    let mut command = vec![
        "tools/phpstan/vendor/bin/phpstan".to_string(),
        "analyse".to_string(),
        "-c".to_string(),
        "./tools/phpstan/phpstan.neon".to_string(),
    ];
    command.extend(user.iter().cloned());
    exec_php(&["opcache.enable_cli=0", "memory_limit=-1"], &command)
}

pub fn phpunit_args(user: &[String]) -> Vec<String> {
    let mut command = vec!["bin/phpunit".to_string(), "--colors=always".to_string()];
    command.extend(user.iter().cloned());
    exec_php(
        &[
            "opcache.enable_cli=0",
            "memory_limit=-1",
            "pcov.enabled=1",
            "xdebug.mode=debug,develop,trace",
            "xdebug.client_port=9003",
            "xdebug.client_host=172.17.0.1",
        ],
        &command,
    )
}

pub fn rector_args(user: &[String]) -> Vec<String> {
    let mut command = vec!["tools/rector/vendor/bin/rector".to_string()];
    command.extend(user.iter().cloned());
    exec_php(&["opcache.enable_cli=0", "memory_limit=-1"], &command)
}

pub fn phpmd_args() -> Vec<String> {
    exec_php(
        &["opcache.enable_cli=0", "memory_limit=-1"],
        &[
            "tools/phpmd/vendor/bin/phpmd".into(),
            "src/".into(),
            "text".into(),
            "cleancode,codesize,controversial,design,naming,unusedcode".into(),
            "--exclude".into(),
            "tests/*,vendor/*,src/enum/*".into(),
            "--baseline-file".into(),
            "phpmd.baseline.xml".into(),
        ],
    )
}

pub fn phan_args(root: &Path, user: &[String]) -> Result<Vec<String>> {
    let base = exec_php(
        &["opcache.enable_cli=0", "memory_limit=-1"],
        &[
            "tools/phan/vendor/bin/phan".into(),
            "--config-file".into(),
            "tools/phan/.phan/config.php".into(),
        ],
    );
    if user.is_empty() || user.iter().any(|arg| explicit_file_selection(arg)) {
        let mut args = base;
        args.extend(user.iter().cloned());
        return Ok(args);
    }
    let mut options = Vec::new();
    let mut php_files = Vec::new();
    let mut index = 0;
    while index < user.len() {
        let arg = &user[index];
        if explicit_file_selection(arg) {
            let mut args = base;
            args.extend(user.iter().cloned());
            return Ok(args);
        }
        if option_takes_value(arg) {
            let Some(value) = user.get(index + 1) else {
                bail!("phan: missing argument for {arg}");
            };
            options.push(arg.clone());
            options.push(value.clone());
            index += 2;
            continue;
        }
        if arg.starts_with('-') {
            options.push(arg.clone());
            index += 1;
            continue;
        }
        if arg.ends_with(".php") {
            php_files.push(normalize_php_path(arg));
            index += 1;
            continue;
        }
        let mut args = base;
        args.extend(user.iter().cloned());
        return Ok(args);
    }
    if php_files.is_empty() {
        let mut args = base;
        args.extend(user.iter().cloned());
        return Ok(args);
    }
    let list_dir = root.join("app").join("var");
    std::fs::create_dir_all(&list_dir)
        .with_context(|| format!("could not create {}", list_dir.display()))?;
    let list_path = list_dir.join("phan-file-list.txt");
    std::fs::write(&list_path, php_files.join("\n") + "\n")
        .with_context(|| format!("could not write {}", list_path.display()))?;
    let mut args = base;
    args.extend(options);
    args.push("--include-analysis-file-list".to_string());
    args.push(php_files.join(","));
    Ok(args)
}

fn normalize_php_path(path: &str) -> String {
    path.strip_prefix("app/")
        .or_else(|| path.strip_prefix("./"))
        .unwrap_or(path)
        .to_string()
}

fn explicit_file_selection(arg: &str) -> bool {
    matches!(
        arg,
        "-r" | "--file-list-only" | "-I" | "--include-analysis-file-list" | "-f" | "--file-list"
    )
}

fn option_takes_value(arg: &str) -> bool {
    matches!(
        arg,
        "-k" | "-f"
            | "-l"
            | "-3"
            | "-I"
            | "-d"
            | "-r"
            | "-o"
            | "-m"
            | "-B"
            | "--config-file"
            | "--file-list"
            | "--directory"
            | "--exclude-directory-list"
            | "--include-analysis-file-list"
            | "--project-root-directory"
            | "--file-list-only"
            | "--output"
            | "--output-mode"
            | "--load-baseline"
            | "--save-baseline"
            | "--processes"
            | "--daemonize-tcp-host"
            | "--daemonize-tcp-port"
            | "--init-level"
            | "--init-analyze-dir"
            | "--init-analyze-file"
    )
}

pub fn run_console(project: &Project, user: &[String]) -> Result<i32> {
    run_status(project, &console_args(project.dev, user))
}

pub fn run_composer(project: &Project, user: &[String]) -> Result<i32> {
    run_status(project, &composer_args(user))
}

pub fn run_tool(project: &Project, args: &[String]) -> Result<i32> {
    run_status(project, args)
}

pub fn symfony_env(project: &Project) -> &str {
    project.app_env.as_str()
}

pub fn fixtures_args(env: &str) -> Vec<String> {
    console_args(
        true,
        &[
            "d:f:l".into(),
            format!("--env={env}"),
            "--no-interaction".into(),
            "--no-debug".into(),
        ],
    )
}

pub fn run_fixtures(project: &Project) -> Result<i32> {
    let mut args = fixtures_args(&project.app_env);
    if !project.dev {
        insert_www_data(&mut args);
    }
    run_status(project, &args)
}

fn insert_www_data(args: &mut Vec<String>) {
    if let Some(index) = args.iter().position(|arg| arg == "exec") {
        args.insert(index + 1, "www-data".to_string());
        args.insert(index + 1, "-u".to_string());
    }
}

pub fn force_drop_database(project: &Project) -> Result<i32> {
    let output = run_output(
        project,
        &[
            "compose".into(),
            "exec".into(),
            "postgres".into(),
            "env".into(),
        ],
    )?;
    if !output.status.success() {
        return Ok(output.status.code().unwrap_or(1));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let Some(db) = text
        .lines()
        .find_map(|line| line.strip_prefix("POSTGRES_DB="))
    else {
        bail!("POSTGRES_DB is not set in the postgres service");
    };
    let mut name = db.trim().to_string();
    if project.app_env == "test" {
        name.push_str("_1");
    }
    let _ = run_status(
        project,
        &[
            "compose".into(),
            "exec".into(),
            "postgres".into(),
            "dropdb".into(),
            "-f".into(),
            name,
        ],
    )?;
    Ok(0)
}

pub fn reset_database(project: &Project) -> Result<i32> {
    let _ = force_drop_database(project)?;
    for command in ["d:d:d", "d:d:c", "d:m:m"] {
        let mut user = vec![format!("--env={}", project.app_env), command.to_string()];
        if command == "d:d:d" {
            user.extend([
                "--force".into(),
                "--if-exists".into(),
                "--no-interaction".into(),
            ]);
        } else {
            user.push("--no-interaction".into());
        }
        let code = run_console(project, &user)?;
        if code != 0 {
            return Ok(code);
        }
    }
    run_fixtures(project)
}

pub fn installer(project: &Project) -> Result<i32> {
    for args in [
        vec!["compose".into(), "kill".into()],
        vec!["compose".into(), "down".into()],
    ] {
        let code = run_status(project, &args)?;
        if code != 0 {
            return Ok(code);
        }
    }
    let code = run_composer(project, &["install".into()])?;
    if code != 0 {
        return Ok(code);
    }
    let code = run_status(project, &["compose".into(), "up".into(), "-d".into()])?;
    if code != 0 {
        return Ok(code);
    }
    std::thread::sleep(std::time::Duration::from_secs(2));
    let code = crate::cache::clear(project)?;
    if code != 0 {
        return Ok(code);
    }
    let code = run_console(project, &["messenger:stop-workers".into()])?;
    if code != 0 {
        return Ok(code);
    }
    reset_database(project)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prod_console_runs_php_before_defines() {
        let args = console_args(false, &["debug:router".into()]);
        assert_eq!(
            args,
            vec![
                "compose",
                "exec",
                "-u",
                "www-data",
                "php",
                "php",
                "-d",
                "opcache.enable_cli=0",
                "-d",
                "memory_limit=8G",
                "bin/console",
                "debug:router",
            ]
        );
    }

    #[test]
    fn dev_console_skips_www_data() {
        let args = console_args(true, &["list".into()]);
        assert!(!args.iter().any(|arg| arg == "-u"));
        let php_at: Vec<_> = args
            .iter()
            .enumerate()
            .filter(|(_, arg)| arg.as_str() == "php")
            .map(|(index, _)| index)
            .collect();
        assert!(php_at.len() >= 2);
        assert_eq!(args[php_at[1] + 1], "-d");
    }

    #[test]
    fn composer_install_keeps_optimizer_flags() {
        let args = composer_args(&["install".into(), "--no-dev".into()]);
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--optimize-autoloader", "--classmap-authoritative"])
        );
        assert!(args.iter().any(|arg| arg == "--profile"));
        assert_eq!(args.last().map(String::as_str), Some("--no-dev"));
    }

    #[test]
    fn composer_update_does_not_add_install_flags() {
        let args = composer_args(&["update".into()]);
        assert!(!args.iter().any(|arg| arg == "--classmap-authoritative"));
    }

    #[test]
    fn phan_collects_php_paths_into_an_include_list() {
        let dir = tempfile::tempdir().unwrap();
        let args = phan_args(
            dir.path(),
            &["app/src/Kernel.php".into(), "./src/User.php".into()],
        )
        .unwrap();
        assert!(args.iter().any(|arg| arg == "--include-analysis-file-list"));
        assert_eq!(
            args.last().map(String::as_str),
            Some("src/Kernel.php,src/User.php")
        );
        let list = std::fs::read_to_string(dir.path().join("app/var/phan-file-list.txt")).unwrap();
        assert!(list.contains("src/Kernel.php"));
    }
}
