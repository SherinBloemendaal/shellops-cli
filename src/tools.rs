use anyhow::{Context, Result, bail};
use comfy_table::{Attribute, Cell, Color};
use std::path::Path;
use std::process::Command;

use crate::build;
use crate::project::Project;
use crate::ui::{self, Theme};

pub fn debug(project: &Project) -> Result<i32> {
    let dotenv = crate::project::read_dotenv(&project.root.join(".env"));
    let version = crate::project::version_value(&project.root, &dotenv);
    let plan = build::plan(&project.root).ok();
    let prefix = plan
        .as_ref()
        .map(|plan| plan.prefix.clone())
        .unwrap_or_default();
    let order = plan
        .as_ref()
        .map(|plan| {
            plan.services
                .iter()
                .map(|service| service.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let secrets = if project.root.join(".docker-secrets.env").is_file() {
        "present"
    } else {
        "absent"
    };
    let uid = rustix::process::getuid().as_raw().to_string();
    let gid = rustix::process::getgid().as_raw().to_string();
    let user = std::env::var("USER").unwrap_or_else(|_| "-".to_string());
    let theme = Theme::stdout();
    println!(
        "{}",
        ui::panel(
            theme,
            vec![
                ("root", project.root.display().to_string()),
                ("APP_ENV", project.app_env.clone()),
                ("mode", if project.dev { "dev" } else { "prod" }.to_string()),
                ("UID", uid),
                ("GID", gid),
                ("USER", user),
                ("VERSION", version),
                ("image prefix", prefix),
                ("build order", order),
                ("docker secrets", secrets.to_string()),
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
    println!("Private and public keys generated in {}", dir.display());
    println!("Passphrase used: {passphrase}");
    println!("Random string for encryption key: {encryption}");
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
    println!("Generated random string: {filtered}");
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
    let theme = Theme::stdout();
    let mut table = ui::table(theme, &["key", "scope", "value"]);
    for (key, hit) in rows {
        let scope = match hit.scope {
            crate::config::Scope::Project => "project",
            crate::config::Scope::Global => "global",
            crate::config::Scope::Inferred => "default",
        };
        table.add_row(vec![
            theme.cell(key, Some(Color::Cyan), &[Attribute::Bold]),
            Cell::new(scope),
            Cell::new(crate::config::display_value(hit)),
        ]);
    }
    println!("{table}");
}
