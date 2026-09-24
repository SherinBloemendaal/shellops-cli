use anyhow::{Context, Result, bail};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::compose;
use crate::project::{self, Project};
use crate::ui;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServicePlan {
    pub name: String,
    pub dockerfile: String,
    pub args: Vec<(String, String)>,
    pub secrets: Vec<String>,
    pub tag: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildPlan {
    pub prefix: String,
    pub services: Vec<ServicePlan>,
}

pub fn plan(root: &Path) -> Result<BuildPlan> {
    let files = dockerfile_bodies(root)?;
    let names: Vec<String> = files.iter().map(|(name, _)| name.clone()).collect();
    let order = build_order(&files)?;
    let dotenv = project::read_dotenv(&root.join(".env"));
    let prefix = image_prefix(root, &names, project_dev());
    let computed = computed_values(root, &dotenv);
    let proc = std::env::vars().collect::<BTreeMap<_, _>>();
    let secrets = secret_names(&root.join(".docker-secrets.env"));
    let mut services = Vec::new();
    for name in order {
        let body = files
            .iter()
            .find(|(service, _)| service == &name)
            .map(|(_, body)| body.as_str())
            .unwrap_or("");
        let declared = declared_args(body);
        let mut args = Vec::new();
        for arg in declared {
            if let Some(value) = merged_value(&arg, &proc, &dotenv, &computed) {
                args.push((arg, value));
            }
        }
        let tag = if prefix.is_empty() {
            format!("{name}:latest")
        } else {
            format!("{prefix}/{name}:latest")
        };
        services.push(ServicePlan {
            name: name.clone(),
            dockerfile: format!("{name}.Dockerfile"),
            args,
            secrets: secrets.clone(),
            tag,
        });
    }
    Ok(BuildPlan { prefix, services })
}

fn project_dev() -> bool {
    std::env::var("APP_ENV").ok().as_deref() == Some("dev")
}

pub fn build_order(files: &[(String, String)]) -> Result<Vec<String>> {
    let names: BTreeSet<String> = files.iter().map(|(name, _)| name.clone()).collect();
    let mut deps: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (name, body) in files {
        let mut set = BTreeSet::new();
        for other in &names {
            if other == name {
                continue;
            }
            let needle = format!("/{other}:");
            if body.contains(&needle) {
                set.insert(other.clone());
            }
        }
        deps.insert(name.clone(), set);
    }
    let mut incoming: BTreeMap<String, usize> =
        names.iter().map(|name| (name.clone(), 0)).collect();
    for (name, set) in &deps {
        if let Some(count) = incoming.get_mut(name) {
            *count = set.len();
        }
    }
    let mut ready: BTreeSet<String> = incoming
        .iter()
        .filter(|(_, count)| **count == 0)
        .map(|(name, _)| name.clone())
        .collect();
    let mut ordered = Vec::new();
    while let Some(next) = ready.iter().next().cloned() {
        ready.remove(&next);
        ordered.push(next.clone());
        for (name, set) in &deps {
            if set.contains(&next)
                && let Some(count) = incoming.get_mut(name)
            {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    ready.insert(name.clone());
                }
            }
        }
    }
    if ordered.len() != names.len() {
        bail!("Dockerfile references contain a cycle");
    }
    Ok(ordered)
}

pub fn declared_args(body: &str) -> Vec<String> {
    let mut args = Vec::new();
    for line in body.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("ARG ") else {
            continue;
        };
        let name = rest.split(['=', ' ', '\t']).next().unwrap_or("").trim();
        if is_ident(name) && !args.iter().any(|existing| existing == name) {
            args.push(name.to_string());
        }
    }
    args
}

fn is_ident(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

pub fn merged_value(
    name: &str,
    proc: &BTreeMap<String, String>,
    dotenv: &BTreeMap<String, String>,
    computed: &BTreeMap<String, String>,
) -> Option<String> {
    for source in [proc, dotenv, computed] {
        if let Some(value) = source.get(name)
            && !value.is_empty()
        {
            return Some(value.clone());
        }
    }
    None
}

pub fn image_prefix(root: &Path, services: &[String], dev: bool) -> String {
    let mut files = if dev {
        vec!["compose.override.yml", "compose.yml", "compose.prod.yml"]
    } else {
        vec!["compose.prod.yml", "compose.yml", "compose.override.yml"]
    };
    files.retain(|name| root.join(name).is_file());
    for name in files {
        let Ok(text) = std::fs::read_to_string(root.join(name)) else {
            continue;
        };
        for line in text.lines() {
            let Some(image) = image_value(line) else {
                continue;
            };
            if let Some(prefix) = prefix_for_service(&image, services) {
                return prefix;
            }
        }
    }
    String::new()
}

fn image_value(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let rest = trimmed.strip_prefix("image:")?;
    let value = rest.trim().trim_matches('"').trim_matches('\'');
    (!value.is_empty()).then(|| value.to_string())
}

fn prefix_for_service(image: &str, services: &[String]) -> Option<String> {
    let image = image.split_whitespace().next().unwrap_or(image);
    for service in services {
        let needle = format!("/{service}");
        let Some(index) = image.find(&needle) else {
            continue;
        };
        let after = &image[index + needle.len()..];
        let boundary = after.is_empty()
            || after.starts_with(':')
            || after.starts_with('@')
            || after.starts_with('$')
            || after.starts_with('}');
        if boundary {
            let prefix = &image[..index];
            if !prefix.is_empty() {
                return Some(prefix.to_string());
            }
        }
    }
    None
}

pub fn secret_names(path: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if is_ident(key) && !value.is_empty() && !names.iter().any(|name| name == key) {
            names.push(key.to_string());
        }
    }
    names
}

fn computed_values(root: &Path, dotenv: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    map.insert(
        "UID".to_string(),
        rustix::process::getuid().as_raw().to_string(),
    );
    map.insert(
        "GID".to_string(),
        rustix::process::getgid().as_raw().to_string(),
    );
    if let Ok(user) = std::env::var("USER")
        && !user.is_empty()
    {
        map.insert("USER".to_string(), user);
    } else if let Ok(user) = std::env::var("USERNAME")
        && !user.is_empty()
    {
        map.insert("USER".to_string(), user);
    }
    map.insert("VERSION".to_string(), project::version_value(root, dotenv));
    map
}

fn dockerfile_bodies(root: &Path) -> Result<Vec<(String, String)>> {
    let mut files = Vec::new();
    for entry in
        std::fs::read_dir(root).with_context(|| format!("could not read {}", root.display()))?
    {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(service) = name.strip_suffix(".Dockerfile") else {
            continue;
        };
        if service.is_empty() || service.contains('/') {
            continue;
        }
        let body = std::fs::read_to_string(entry.path())
            .with_context(|| format!("could not read {}", entry.path().display()))?;
        files.push((service.to_string(), body));
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(files)
}

pub fn build(project: &Project) -> Result<i32> {
    let plan = plan(&project.root)?;
    if plan.services.is_empty() {
        bail!("no *.Dockerfile files in {}", project.root.display());
    }
    if plan.prefix.is_empty() {
        bail!("could not find an image prefix in the compose files");
    }
    let secret_env = secret_env(&project.root.join(".docker-secrets.env"))?;
    for service in &plan.services {
        ui::section(&format!("Building {}", service.name));
        let mut args = vec!["build".to_string()];
        for secret in &service.secrets {
            args.push("--secret".to_string());
            args.push(format!("id={secret},env={secret}"));
        }
        for (key, value) in &service.args {
            args.push("--build-arg".to_string());
            args.push(format!("{key}={value}"));
        }
        args.push("-t".to_string());
        args.push(service.tag.clone());
        args.push("-f".to_string());
        args.push(service.dockerfile.clone());
        args.push(".".to_string());
        let mut command = std::process::Command::new("docker");
        command.args(&args).current_dir(&project.root);
        command.env("DOCKER_BUILDKIT", "1");
        command.env("COMPOSE_DOCKER_CLI_BUILD", "1");
        compose::apply_env(&mut command);
        for (key, value) in &secret_env {
            command.env(key, value);
        }
        let status = command.status().context("could not run docker build")?;
        let code = status.code().unwrap_or(1);
        if code != 0 {
            return Ok(code);
        }
    }
    ui::ok("images tagged latest");
    Ok(0)
}

fn secret_env(path: &Path) -> Result<Vec<(String, String)>> {
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("could not read {}", path.display()))?;
    let mut pairs = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=')
            && is_ident(key.trim())
        {
            pairs.push((key.trim().to_string(), value.to_string()));
        }
    }
    Ok(pairs)
}

pub fn dockerfile_path(root: &Path, service: &str) -> PathBuf {
    root.join(format!("{service}.Dockerfile"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundler_is_built_before_images_that_copy_it() {
        let files = vec![
            (
                "php".to_string(),
                "FROM example/bundler:latest AS bundler\nFROM example/vue:latest AS vue\n"
                    .to_string(),
            ),
            (
                "vue".to_string(),
                "FROM example/bundler:latest AS bundler\n".to_string(),
            ),
            ("bundler".to_string(), "FROM alpine:latest\n".to_string()),
        ];
        assert_eq!(build_order(&files).unwrap(), vec!["bundler", "vue", "php"]);
    }

    #[test]
    fn build_args_keep_only_declared_names() {
        let body = "ARG FOO\nARG UID=1000\nARG VERSION\nRUN echo hi\n";
        assert_eq!(declared_args(body), vec!["FOO", "UID", "VERSION"]);
        let proc = BTreeMap::from([("FOO".to_string(), "from-env".to_string())]);
        let dotenv = BTreeMap::from([
            ("FOO".to_string(), "from-file".to_string()),
            ("SECRET".to_string(), "nope".to_string()),
        ]);
        let computed = BTreeMap::from([
            ("UID".to_string(), "501".to_string()),
            ("VERSION".to_string(), "latest".to_string()),
        ]);
        assert_eq!(
            merged_value("FOO", &proc, &dotenv, &computed).as_deref(),
            Some("from-env")
        );
        assert_eq!(
            merged_value("UID", &BTreeMap::new(), &dotenv, &computed).as_deref(),
            Some("501")
        );
        assert_eq!(
            merged_value("SECRET", &BTreeMap::new(), &dotenv, &computed).as_deref(),
            Some("nope")
        );
        let declared = declared_args(body);
        let selected: Vec<_> = declared
            .iter()
            .filter_map(|name| {
                merged_value(name, &proc, &dotenv, &computed).map(|value| (name.clone(), value))
            })
            .collect();
        assert!(selected.iter().any(|(key, _)| key == "FOO"));
        assert!(!selected.iter().any(|(key, _)| key == "SECRET"));
    }

    #[test]
    fn prefix_comes_from_the_service_image() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("compose.override.yml"),
            "services:\n  php:\n    image: login.example/php:${VERSION:-latest}\n  redis:\n    image: redis:alpine\n",
        )
        .unwrap();
        assert_eq!(
            image_prefix(dir.path(), &["php".to_string(), "vue".to_string()], true),
            "login.example"
        );
    }

    #[test]
    fn secrets_file_yields_names_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".docker-secrets.env");
        std::fs::write(&path, "# comment\nNPM_TOKEN=super-secret\n\n").unwrap();
        assert_eq!(secret_names(&path), vec!["NPM_TOKEN".to_string()]);
    }
}
