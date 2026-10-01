use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use toml::Value;

use crate::project::{git_root, shellops_home};

const SECRET_MASK: &str = "********";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Project,
    Global,
    Inferred,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Plain,
    Secret,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub value: String,
    pub scope: Scope,
    pub visibility: Visibility,
}

#[derive(Debug, Clone)]
pub struct Store {
    pub dir: PathBuf,
}

impl Store {
    pub fn open() -> Result<Self> {
        Ok(Self {
            dir: shellops_home()?,
        })
    }

    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn get(&self, key: &str, project: Option<&Path>) -> Result<Hit> {
        let hit = self
            .lookup(key, project)?
            .with_context(|| format!("config key {key} is not set"))?;
        if hit.visibility == Visibility::Secret {
            bail!("refusing to print secret {key}");
        }
        Ok(hit)
    }

    pub fn lookup(&self, key: &str, project: Option<&Path>) -> Result<Option<Hit>> {
        self.lookup_with(key, project, inferred_default(key))
    }

    pub fn lookup_with(
        &self,
        key: &str,
        project: Option<&Path>,
        inferred: Option<String>,
    ) -> Result<Option<Hit>> {
        let config = read_value(&self.dir.join("config.toml"))?;
        let secrets = read_value(&self.dir.join("secrets.toml"))?;
        if let Some(root) = project
            && let Some(hit) = layer(&secrets, &config, key, root, Scope::Project)
        {
            return Ok(Some(hit));
        }
        if let Some(hit) = layer(&secrets, &config, key, Path::new(""), Scope::Global) {
            return Ok(Some(hit));
        }
        Ok(inferred.map(|value| Hit {
            value,
            scope: Scope::Inferred,
            visibility: Visibility::Plain,
        }))
    }

    pub fn set(&self, key: &str, value: &str, secret: bool, project: Option<&Path>) -> Result<()> {
        validate_key(key)?;
        fs::create_dir_all(&self.dir)
            .with_context(|| format!("could not create {}", self.dir.display()))?;
        restrict_dir(&self.dir)?;
        let target_name = if secret {
            "secrets.toml"
        } else {
            "config.toml"
        };
        let other_name = if secret {
            "config.toml"
        } else {
            "secrets.toml"
        };
        self.write_key(target_name, key, Some(value), project)?;
        self.write_key(other_name, key, None, project)?;
        Ok(())
    }

    pub fn unset(&self, key: &str, project: Option<&Path>) -> Result<()> {
        validate_key(key)?;
        self.write_key("config.toml", key, None, project)?;
        self.write_key("secrets.toml", key, None, project)?;
        Ok(())
    }

    pub fn list(&self, project: Option<&Path>) -> Result<Vec<(String, Hit)>> {
        let config = read_value(&self.dir.join("config.toml"))?;
        let secrets = read_value(&self.dir.join("secrets.toml"))?;
        let mut keys = BTreeMap::new();
        collect_keys(&config, "", Scope::Global, Visibility::Plain, &mut keys);
        collect_keys(&secrets, "", Scope::Global, Visibility::Secret, &mut keys);
        if let Some(root) = project {
            if let Some(table) = project_table(&config, root) {
                collect_keys(table, "", Scope::Project, Visibility::Plain, &mut keys);
            }
            if let Some(table) = project_table(&secrets, root) {
                collect_keys(table, "", Scope::Project, Visibility::Secret, &mut keys);
            }
        }
        let mut rows = Vec::new();
        for key in keys.into_keys() {
            if let Some(hit) = self.lookup(&key, project)? {
                rows.push((key, hit));
            }
        }
        Ok(rows)
    }

    fn write_key(
        &self,
        file_name: &str,
        key: &str,
        value: Option<&str>,
        project: Option<&Path>,
    ) -> Result<()> {
        let path = self.dir.join(file_name);
        let mut root = read_value(&path)?;
        let destination = match project {
            Some(project) => project_table_mut(&mut root, project),
            None => match &mut root {
                Value::Table(table) => table,
                _ => bail!("config root must be a table"),
            },
        };
        assign(destination, key, value);
        if path.exists() || value.is_some() {
            write_private(&path, &toml::to_string_pretty(&root).unwrap_or_default())?;
        }
        Ok(())
    }
}

pub fn display_value(hit: &Hit) -> String {
    if hit.visibility == Visibility::Secret {
        SECRET_MASK.to_string()
    } else {
        hit.value.clone()
    }
}

pub fn inferred_default(key: &str) -> Option<String> {
    match key {
        "openrouter.model" => Some("openai/gpt-4o-mini".to_string()),
        _ => None,
    }
}

pub fn project_key() -> Result<PathBuf> {
    git_root()
}

fn validate_key(key: &str) -> Result<()> {
    if key.is_empty()
        || key.starts_with('.')
        || key.ends_with('.')
        || key.split('.').any(|part| part.is_empty())
    {
        bail!("invalid config key {key}");
    }
    if key == "project" || key.starts_with("project.") {
        bail!("config key project is reserved");
    }
    Ok(())
}

fn read_value(path: &Path) -> Result<Value> {
    if !path.is_file() {
        return Ok(Value::Table(Default::default()));
    }
    let text =
        fs::read_to_string(path).with_context(|| format!("could not read {}", path.display()))?;
    if text.trim().is_empty() {
        return Ok(Value::Table(Default::default()));
    }
    toml::from_str(&text).with_context(|| format!("could not parse {}", path.display()))
}

fn layer(secrets: &Value, config: &Value, key: &str, project: &Path, scope: Scope) -> Option<Hit> {
    let (secret_table, plain_table) = if scope == Scope::Project {
        (
            project_table(secrets, project),
            project_table(config, project),
        )
    } else {
        (Some(secrets), Some(config))
    };
    if let Some(value) = secret_table.and_then(|table| dig(table, key)) {
        return Some(Hit {
            value,
            scope,
            visibility: Visibility::Secret,
        });
    }
    plain_table
        .and_then(|table| dig(table, key))
        .map(|value| Hit {
            value,
            scope,
            visibility: Visibility::Plain,
        })
}

fn project_table<'a>(root: &'a Value, project: &Path) -> Option<&'a Value> {
    root.get("project")?.get(project_name(project))
}

fn project_table_mut<'a>(
    root: &'a mut Value,
    project: &Path,
) -> &'a mut toml::map::Map<String, Value> {
    let root_table = root.as_table_mut().expect("root table");
    let projects = root_table
        .entry("project")
        .or_insert_with(|| Value::Table(Default::default()));
    let projects = projects.as_table_mut().expect("project table");
    let entry = projects
        .entry(project_name(project))
        .or_insert_with(|| Value::Table(Default::default()));
    entry.as_table_mut().expect("project scope")
}

fn project_name(project: &Path) -> String {
    project.to_string_lossy().to_string()
}

fn dig(root: &Value, key: &str) -> Option<String> {
    let mut current = root;
    for part in key.split('.') {
        current = current.get(part)?;
    }
    value_string(current)
}

fn value_string(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Integer(number) => Some(number.to_string()),
        Value::Float(number) => Some(number.to_string()),
        Value::Boolean(flag) => Some(flag.to_string()),
        Value::Datetime(stamp) => Some(stamp.to_string()),
        Value::Array(_) | Value::Table(_) => None,
    }
}

fn assign(table: &mut toml::map::Map<String, Value>, key: &str, value: Option<&str>) {
    let mut parts = key.split('.').collect::<Vec<_>>();
    let Some(leaf) = parts.pop() else {
        return;
    };
    let mut current = table;
    for part in parts {
        let entry = current
            .entry(part.to_string())
            .or_insert_with(|| Value::Table(Default::default()));
        if !entry.is_table() {
            *entry = Value::Table(Default::default());
        }
        current = entry.as_table_mut().expect("nested table");
    }
    match value {
        Some(value) => {
            current.insert(leaf.to_string(), Value::String(value.to_string()));
        }
        None => {
            current.remove(leaf);
        }
    }
}

fn collect_keys(
    value: &Value,
    prefix: &str,
    scope: Scope,
    visibility: Visibility,
    out: &mut BTreeMap<String, Hit>,
) {
    let Some(table) = value.as_table() else {
        return;
    };
    for (key, child) in table {
        if prefix.is_empty() && scope == Scope::Global && key == "project" {
            continue;
        }
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        if child.is_table() {
            collect_keys(child, &path, scope, visibility, out);
        } else if let Some(text) = value_string(child) {
            out.insert(
                path,
                Hit {
                    value: text,
                    scope,
                    visibility,
                },
            );
        }
    }
}

fn write_private(path: &Path, body: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        restrict_dir(parent)?;
    }
    fs::write(path, body).with_context(|| format!("could not write {}", path.display()))?;
    restrict_file(path)
}

fn restrict_dir(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn restrict_file(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_beats_global_beats_inferred_and_secrets_stay_hidden() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::at(dir.path());
        let project = Path::new("/work/auth");
        store
            .set("openrouter.model", "global-model", false, None)
            .unwrap();
        store
            .set("openrouter.model", "project-model", false, Some(project))
            .unwrap();
        store.set("github", "token-value", true, None).unwrap();
        let hit = store
            .lookup("openrouter.model", Some(project))
            .unwrap()
            .unwrap();
        assert_eq!(hit.value, "project-model");
        assert_eq!(hit.scope, Scope::Project);
        let global = store.lookup("openrouter.model", None).unwrap().unwrap();
        assert_eq!(global.value, "global-model");
        store.unset("openrouter.model", None).unwrap();
        store.unset("openrouter.model", Some(project)).unwrap();
        let inferred = store
            .lookup_with("openrouter.model", None, Some("inferred-model".into()))
            .unwrap()
            .unwrap();
        assert_eq!(inferred.scope, Scope::Inferred);
        assert_eq!(inferred.value, "inferred-model");
        let err = store.get("github", None).unwrap_err();
        assert!(err.to_string().contains("refusing to print secret"));
        let listed = store.list(None).unwrap();
        let github = listed.iter().find(|(key, _)| key == "github").unwrap();
        assert_eq!(display_value(&github.1), "********");
        let mode = fs::metadata(dir.path().join("secrets.toml"))
            .unwrap()
            .permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(mode.mode() & 0o777, 0o600);
        }
    }
}
