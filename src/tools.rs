use anyhow::{Context, Result, bail};
use comfy_table::{Attribute, Cell, Color};

use crate::ui::{self, Align, Sheet, Theme};
use std::path::Path;
use std::process::Command;

use crate::build;
use crate::project::Project;

pub fn debug(project: &Project) -> Result<i32> {
    let dotenv = crate::project::read_dotenv(&project.root.join(".env"));
    let version = crate::project::version_value(&project.root, &dotenv);
    let plan = build::plan(&project.root).ok();
    let prefix = plan
        .as_ref()
        .map(|plan| plan.prefix.clone())
        .unwrap_or_default();
    let theme = Theme::stdout();
    let order = plan
        .as_ref()
        .map(|plan| {
            plan.services
                .iter()
                .map(|service| service.name.as_str())
                .collect::<Vec<_>>()
                .join(&format!(" {} ", theme.icons().arrow))
        })
        .filter(|order| !order.is_empty())
        .unwrap_or_else(|| "-".to_string());
    let secrets = if project.root.join(".docker-secrets.env").is_file() {
        ".docker-secrets.env"
    } else {
        "-"
    };
    let ids = format!(
        "{}:{}",
        rustix::process::getuid().as_raw(),
        rustix::process::getgid().as_raw()
    );
    let user = std::env::var("USER").unwrap_or_else(|_| "-".to_string());
    let mode = if project.dev { "dev" } else { "prod" };
    let app_env = if project.app_env == mode {
        theme.cell(&project.app_env, None, &[])
    } else {
        theme.cell(
            format!("{}{}{mode} mode", project.app_env, theme.icons().sep),
            None,
            &[],
        )
    };
    let prefix = if prefix.is_empty() {
        "-".to_string()
    } else {
        prefix
    };
    println!(
        "{}",
        ui::panel(
            theme,
            vec![
                ("root", theme.token_cell(&ui::tilde(&project.root))),
                ("APP_ENV", app_env),
                ("USER", theme.cell(&user, None, &[])),
                ("UID:GID", theme.cell(&ids, None, &[])),
                ("VERSION", theme.cell(&version, None, &[])),
                ("image prefix", theme.token_cell(&prefix)),
                ("build order", theme.cell(order, None, &[])),
                ("docker secrets", theme.token_cell(secrets)),
            ],
        )
    );
    Ok(0)
}

pub fn sort_dotenv(path: &Path) -> Result<i32> {
    if !path.is_file() {
        bail!("{} is not a file", path.display());
    }
    let path_text = path.to_string_lossy().to_string();
    let status = Command::new("sort")
        .args(["-b", "-o", &path_text, &path_text])
        .status()
        .context("could not run sort")?;
    Ok(status.code().unwrap_or(1))
}

pub fn keypair(dir: &Path, suffix: Option<&str>) -> Result<i32> {
    if !dir.is_dir() {
        bail!("{} is not a directory", dir.display());
    }
    let suffix = suffix
        .filter(|value| !value.is_empty())
        .map(|value| format!("-{value}"))
        .unwrap_or_default();
    let passphrase = openssl_rand()?;
    let encryption = openssl_rand()?;
    let private = dir.join(format!("private{suffix}.pem"));
    let public = dir.join(format!("public{suffix}.pem"));
    let private_text = private.to_string_lossy().to_string();
    let public_text = public.to_string_lossy().to_string();
    let passout = format!("pass:{passphrase}");
    let status = Command::new("openssl")
        .args([
            "genrsa",
            "-aes128",
            "-passout",
            &passout,
            "-out",
            &private_text,
            "2048",
        ])
        .status()
        .context("could not run openssl")?;
    if !status.success() {
        return Ok(status.code().unwrap_or(1));
    }
    let passin = format!("pass:{passphrase}");
    let status = Command::new("openssl")
        .args([
            "rsa",
            "-in",
            &private_text,
            "-passin",
            &passin,
            "-pubout",
            "-out",
            &public_text,
        ])
        .status()
        .context("could not run openssl")?;
    if !status.success() {
        return Ok(status.code().unwrap_or(1));
    }
    let theme = Theme::stdout();
    let directory = ui::tilde(dir);
    let private_path = ui::tilde(&private);
    let public_path = ui::tilde(&public);
    ui::section("Key pair");
    println!(
        "{}",
        ui::panel(
            theme,
            vec![
                ("directory", theme.token_cell(&directory)),
                ("private", theme.token_cell(&private_path)),
                ("public", theme.token_cell(&public_path)),
                ("passphrase", Cell::new(passphrase)),
                ("encryption", Cell::new(encryption)),
            ],
        )
    );
    Ok(0)
}

pub fn random_string(length: usize) -> Result<i32> {
    if length == 0 {
        bail!("length must be at least 1");
    }
    let output = Command::new("openssl")
        .args(["rand", "-base64", &length.to_string()])
        .output()
        .context("could not run openssl")?;
    if !output.status.success() {
        return Ok(output.status.code().unwrap_or(1));
    }
    let filtered: String = String::from_utf8_lossy(&output.stdout)
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .take(length)
        .collect();
    if ui::stdout_is_tty() {
        ui::ok(&format!("Generated random string: {filtered}"));
    } else {
        println!("{filtered}");
    }
    Ok(0)
}

fn openssl_rand() -> Result<String> {
    let output = Command::new("openssl")
        .args(["rand", "-base64", "32"])
        .output()
        .context("could not run openssl")?;
    if !output.status.success() {
        bail!("openssl rand failed");
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let compact: String = text.chars().filter(|ch| !ch.is_whitespace()).collect();
    if compact.is_empty() {
        bail!("openssl rand returned an empty value");
    }
    Ok(compact)
}

pub fn config_list(rows: &[(String, crate::config::Hit)]) {
    // setup.* is so init's own bookkeeping; so init status shows it properly.
    let rows: Vec<&(String, crate::config::Hit)> = rows
        .iter()
        .filter(|(key, _)| !key.starts_with("setup."))
        .collect();
    if rows.is_empty() {
        ui::info("No settings yet");
        ui::hint("Set one with so config set KEY VALUE, or run so init.");
        return;
    }
    let theme = Theme::stdout();
    let mut sheet = Sheet::new(
        theme,
        &[
            ("key", Align::Left),
            ("scope", Align::Left),
            ("value", Align::Left),
        ],
    )
    .flex(2);
    for (key, hit) in rows {
        let scope = match hit.scope {
            crate::config::Scope::Project => "project",
            crate::config::Scope::Global => "global",
            crate::config::Scope::Inferred => "default",
        };
        let value = crate::config::display_value(hit);
        sheet.row(vec![
            theme.cell(key, Some(Color::Cyan), &[Attribute::Bold]),
            theme.cell(scope, None, &[]),
            theme.token_cell(&value),
        ]);
    }
    println!("{sheet}");
}
