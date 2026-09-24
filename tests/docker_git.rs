use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn so_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_so"))
}

struct Fixture {
    root: PathBuf,
    home: PathBuf,
    bin: PathBuf,
    log: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap().keep();
        let root = dir.join("project");
        let home = dir.join("home");
        let bin = dir.join("bin");
        let log = dir.join("log");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&bin).unwrap();
        fs::write(
            bin.join("docker"),
            r#"#!/bin/sh
log="$SHELLOPS_FAKE_LOG"
printf '%s\n' '---' >> "$log"
for arg in "$@"; do
  printf 'arg:%s\n' "$arg" >> "$log"
done
printf 'env:UID=%s\n' "${UID-}" >> "$log"
printf 'env:GID=%s\n' "${GID-}" >> "$log"
printf 'env:USER=%s\n' "${USER-}" >> "$log"
if [ "$1" = "image" ]; then
  exit 1
fi
exit 0
"#,
        )
        .unwrap();
        fs::write(
            bin.join("git"),
            r#"#!/bin/sh
if [ "$1" = "rev-parse" ] && [ "$2" = "--show-toplevel" ]; then
  printf '%s\n' "$SHELLOPS_FAKE_ROOT"
  exit 0
fi
exit 0
"#,
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for name in ["docker", "git"] {
                fs::set_permissions(bin.join(name), fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        Self {
            root,
            home,
            bin,
            log,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(so_bin());
        let path = format!(
            "{}:{}",
            self.bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        command
            .current_dir(&self.root)
            .env("PATH", path)
            .env("HOME", &self.home)
            .env("SHELLOPS_FAKE_LOG", &self.log)
            .env("SHELLOPS_FAKE_ROOT", &self.root)
            .env("SHELLOPS_NO_UPDATE_CHECK", "1")
            .env_remove("UID")
            .env_remove("GID")
            .env_remove("USER")
            .env_remove("APP_ENV");
        command
    }
}

fn write_compose(root: &Path) {
    fs::write(
        root.join("compose.yml"),
        "services:\n  php:\n    image: example/php:latest\n",
    )
    .unwrap();
    fs::write(
        root.join("compose.override.yml"),
        "services:\n  php:\n    image: login.example/php:${VERSION:-latest}\n  vue:\n    image: login.example/vue:latest\n  bundler:\n    image: login.example/bundler:latest\n",
    )
    .unwrap();
}

fn log_invocations(log: &Path) -> Vec<Vec<String>> {
    let text = fs::read_to_string(log).unwrap_or_default();
    let mut invocations = Vec::new();
    let mut current = Vec::new();
    for line in text.lines() {
        if line == "---" {
            if !current.is_empty() {
                invocations.push(current);
            }
            current = Vec::new();
            continue;
        }
        if let Some(arg) = line.strip_prefix("arg:") {
            current.push(arg.to_string());
        }
    }
    if !current.is_empty() {
        invocations.push(current);
    }
    invocations
}

#[test]
fn compose_injects_uid_gid_and_user() {
    let fixture = Fixture::new();
    write_compose(&fixture.root);
    fs::write(fixture.root.join(".env"), "APP_ENV=prod\n").unwrap();
    let output = fixture.command().arg("compose").arg("up").output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = fs::read_to_string(&fixture.log).unwrap();
    assert!(text.contains("arg:compose\narg:up\n"));
    assert!(
        text.lines()
            .any(|line| line.starts_with("env:UID=") && line != "env:UID=")
    );
    assert!(
        text.lines()
            .any(|line| line.starts_with("env:GID=") && line != "env:GID=")
    );
    assert!(
        text.lines()
            .any(|line| line.starts_with("env:USER=") && line != "env:USER=")
    );
}

#[test]
fn build_orders_images_and_filters_args() {
    let fixture = Fixture::new();
    write_compose(&fixture.root);
    fs::write(
        fixture.root.join(".env"),
        "APP_ENV=dev\nSHELLOPS_FIXTURE_FOO=from-dotenv\nSECRET=hidden-secret\n",
    )
    .unwrap();
    fs::write(
        fixture.root.join(".docker-secrets.env"),
        "NPM_TOKEN=super-secret-value\n",
    )
    .unwrap();
    fs::write(
        fixture.root.join("bundler.Dockerfile"),
        "FROM alpine:latest\nARG SHELLOPS_FIXTURE_FOO\nARG UID\n",
    )
    .unwrap();
    fs::write(
        fixture.root.join("vue.Dockerfile"),
        "FROM login.example/bundler:latest AS bundler\nARG SHELLOPS_FIXTURE_FOO\n",
    )
    .unwrap();
    fs::write(
        fixture.root.join("php.Dockerfile"),
        "FROM login.example/bundler:latest AS bundler\nFROM login.example/vue:latest AS vue\nARG BAR\nARG UID\n",
    )
    .unwrap();
    let output = fixture.command().arg("build").output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let invocations = log_invocations(&fixture.log);
    let builds: Vec<&Vec<String>> = invocations
        .iter()
        .filter(|args| args.first().map(String::as_str) == Some("build"))
        .collect();
    let tags: Vec<&str> = builds
        .iter()
        .filter_map(|args| {
            args.windows(2)
                .find(|pair| pair[0] == "-t")
                .map(|pair| pair[1].as_str())
        })
        .collect();
    assert_eq!(
        tags,
        vec![
            "login.example/bundler:latest",
            "login.example/vue:latest",
            "login.example/php:latest",
        ]
    );
    let bundler = builds[0];
    assert!(
        bundler
            .iter()
            .any(|arg| arg == "SHELLOPS_FIXTURE_FOO=from-dotenv")
    );
    assert!(bundler.iter().any(|arg| arg.starts_with("UID=")));
    assert!(
        !bundler
            .iter()
            .any(|arg| arg.contains("hidden-secret") || arg.starts_with("SECRET="))
    );
    assert!(
        bundler
            .iter()
            .any(|arg| arg == "id=NPM_TOKEN,env=NPM_TOKEN")
    );
    assert!(
        !fs::read_to_string(&fixture.log)
            .unwrap()
            .contains("super-secret-value")
    );
    let php = builds[2];
    assert!(!php.iter().any(|arg| arg.starts_with("BAR=")));
    assert!(
        !php.iter()
            .any(|arg| arg.starts_with("SHELLOPS_FIXTURE_FOO="))
    );
}

#[test]
fn prod_console_argument_order() {
    let fixture = Fixture::new();
    write_compose(&fixture.root);
    fs::write(fixture.root.join(".env"), "APP_ENV=prod\n").unwrap();
    let output = fixture
        .command()
        .args(["console", "debug:router"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let invocations = log_invocations(&fixture.log);
    let args = invocations.last().unwrap();
    assert_eq!(
        args.as_slice(),
        [
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
fn composer_install_keeps_flags() {
    let fixture = Fixture::new();
    write_compose(&fixture.root);
    fs::write(fixture.root.join(".env"), "APP_ENV=dev\n").unwrap();
    let output = fixture
        .command()
        .args(["composer", "install"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let args = log_invocations(&fixture.log).pop().unwrap();
    assert!(args.windows(2).any(|pair| pair[0] == "--optimize-autoloader" && pair[1] == "--classmap-authoritative"));
    assert_eq!(args.last().map(String::as_str), Some("install"));
}
