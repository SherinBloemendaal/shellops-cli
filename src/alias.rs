use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::Store;
use crate::identity::{self, Identity, KeyCheck, Signer};
use comfy_table::{Attribute, Color};

use crate::ui::{self, Theme};

pub const MAX_DEPTH: u32 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Global,
    Dir,
}

impl Scope {
    pub fn flag(self) -> &'static str {
        match self {
            Self::Global => "--global",
            Self::Dir => "--dir",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Dir => "dir",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Roots {
    pub home: PathBuf,
    pub dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AliasFile {
    pub name: String,
    pub body: String,
    pub sha256: String,
    pub author: Author,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Signature>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Author {
    pub github: String,
    pub public_key: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Signature {
    pub sshsig: String,
}

#[derive(Debug, Clone)]
pub struct Record {
    pub scope: Scope,
    pub path: PathBuf,
    pub file: AliasFile,
    pub integrity: Integrity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Integrity {
    Valid,
    Unsigned,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trust {
    OwnKey,
    Author,
    Hash,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunPlan {
    pub body: String,
    pub args: Vec<String>,
    pub depth: u32,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct TrustFile {
    #[serde(default)]
    hash: Vec<HashTrust>,
    #[serde(default)]
    author: Vec<AuthorTrust>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
struct HashTrust {
    sha256: String,
    note: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
struct AuthorTrust {
    github: String,
    fingerprint: String,
}

pub fn validate_name(name: &str) -> Result<()> {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        bail!("alias name is empty");
    };
    if !first.is_ascii_lowercase()
        || !name
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
    {
        bail!("alias name {name} must match [a-z][a-z0-9-]*");
    }
    if reserved(name) {
        bail!("{name} is a built-in command. Pick another alias name.");
    }
    Ok(())
}

pub fn reserved(name: &str) -> bool {
    RESERVED.contains(&name)
}

const RESERVED: &[&str] = &[
    "alias",
    "build",
    "c",
    "cc",
    "compose",
    "composer",
    "config",
    "console",
    "cp",
    "csf",
    "csfixer",
    "debug",
    "dump",
    "f",
    "fdd",
    "fixtures",
    "force-drop-database",
    "github",
    "help",
    "init",
    "installer",
    "keypair",
    "phan",
    "phpmd",
    "phpstan",
    "phpunit",
    "randomstr",
    "rd",
    "rector",
    "release",
    "reset-database",
    "sort-dotenv",
    "update",
    "yarn",
];

pub fn require_scope(global: bool, dir: bool) -> Result<Scope> {
    match (global, dir) {
        (true, false) => Ok(Scope::Global),
        (false, true) => Ok(Scope::Dir),
        (true, true) => bail!("pass --global or --dir, not both"),
        (false, false) => bail!("pass --global or --dir"),
    }
}

pub fn create(
    roots: &Roots,
    store: &Store,
    signer: &Signer,
    scope: Scope,
    name: &str,
    body: &str,
) -> Result<Record> {
    let identity = identity::require(store)?;
    validate_name(name)?;
    let body = require_body(body)?;
    ensure_creatable(roots, scope, name)?;
    write_signed(roots, store, signer, scope, name, &body, &identity)
}

pub fn edit(
    roots: &Roots,
    store: &Store,
    signer: &Signer,
    scope: Scope,
    name: &str,
    body: &str,
) -> Result<Record> {
    let identity = identity::require(store)?;
    let existing = require_one(roots, name, Some(scope))?;
    let body = require_body(body)?;
    if body == existing.file.body {
        return Ok(existing);
    }
    write_signed(roots, store, signer, scope, name, &body, &identity)
}

pub fn rename(roots: &Roots, scope: Scope, old: &str, new: &str) -> Result<Record> {
    if old == new {
        bail!("the new name is the same as the old name");
    }
    validate_name(new)?;
    let existing = require_one(roots, old, Some(scope))?;
    ensure_creatable(roots, scope, new)?;
    let mut file = existing.file;
    file.name = new.to_string();
    let path = alias_path(roots, scope, new);
    write_file(&path, &file, scope)?;
    if existing.path != path {
        let _ = fs::remove_file(&existing.path);
    }
    load_path(scope, path)
}

pub fn remove(roots: &Roots, scope: Scope, name: &str) -> Result<PathBuf> {
    let existing = require_one(roots, name, Some(scope))?;
    fs::remove_file(&existing.path)
        .with_context(|| format!("could not remove {}", existing.path.display()))?;
    Ok(existing.path)
}

pub fn view_records(roots: &Roots, name: &str, scope: Option<Scope>) -> Result<Vec<Record>> {
    let found = find(roots, name, scope)?;
    if found.is_empty() {
        bail!("alias {name} was not found");
    }
    Ok(found)
}

pub fn trust_hash(
    roots: &Roots,
    store: &Store,
    name: &str,
    scope: Option<Scope>,
) -> Result<Record> {
    let record = require_one(roots, name, scope)?;
    if record.integrity == Integrity::Invalid {
        bail!("alias {name} has an invalid signature. Fix it with so alias edit.");
    }
    let mut trust = read_trust(store)?;
    if !trust
        .hash
        .iter()
        .any(|item| item.sha256 == record.file.sha256)
    {
        trust.hash.push(HashTrust {
            sha256: record.file.sha256.clone(),
            note: format!("{name} in {}", record.path.display()),
        });
        write_trust(store, &trust)?;
    }
    Ok(record)
}

pub fn trust_author(
    roots: &Roots,
    store: &Store,
    name: &str,
    scope: Option<Scope>,
) -> Result<Record> {
    let record = require_one(roots, name, scope)?;
    if record.integrity != Integrity::Valid {
        bail!("alias {name} has no valid signature, so its author cannot be trusted");
    }
    match identity::key_status(
        store,
        &record.file.author.github,
        &record.file.author.public_key,
        &identity::keys_base(),
    ) {
        KeyCheck::Listed => {}
        KeyCheck::Missing => bail!(
            "{} is not a GitHub SSH key of {}",
            record.file.author.fingerprint,
            record.file.author.github
        ),
        KeyCheck::Unknown => {
            bail!(
                "could not check GitHub SSH keys for {}",
                record.file.author.github
            )
        }
    }
    let mut trust = read_trust(store)?;
    let entry = AuthorTrust {
        github: record.file.author.github.clone(),
        fingerprint: record.file.author.fingerprint.clone(),
    };
    if !trust.author.contains(&entry) {
        trust.author.push(entry);
        write_trust(store, &trust)?;
    }
    Ok(record)
}

pub fn untrust_hash(
    roots: &Roots,
    store: &Store,
    name: &str,
    scope: Option<Scope>,
) -> Result<Record> {
    let record = require_one(roots, name, scope)?;
    let mut trust = read_trust(store)?;
    let before = trust.hash.len();
    trust.hash.retain(|item| item.sha256 != record.file.sha256);
    if trust.hash.len() == before {
        bail!("hash of {name} is not trusted");
    }
    write_trust(store, &trust)?;
    Ok(record)
}

pub fn untrust_author(
    roots: &Roots,
    store: &Store,
    name: &str,
    scope: Option<Scope>,
) -> Result<Record> {
    let record = require_one(roots, name, scope)?;
    let mut trust = read_trust(store)?;
    let before = trust.author.len();
    trust.author.retain(|item| {
        item.github != record.file.author.github
            || item.fingerprint != record.file.author.fingerprint
    });
    if trust.author.len() == before {
        bail!("author of {name} is not trusted");
    }
    write_trust(store, &trust)?;
    Ok(record)
}

pub fn list_records(roots: &Roots) -> Result<Vec<Record>> {
    let mut records = Vec::new();
    records.extend(read_dir(roots, Scope::Dir)?);
    records.extend(read_dir(roots, Scope::Global)?);
    records.sort_by(|left, right| {
        left.file
            .name
            .cmp(&right.file.name)
            .then(scope_order(left.scope).cmp(&scope_order(right.scope)))
    });
    Ok(records)
}

pub fn plan_run(
    roots: &Roots,
    store: &Store,
    name: &str,
    args: &[String],
    depth: u32,
) -> Result<RunPlan> {
    if depth >= MAX_DEPTH {
        bail!("alias {name} nested more than {MAX_DEPTH} times");
    }
    let found = find(roots, name, None)?;
    if found.is_empty() {
        return Err(ui::hinted(
            format!("unknown command: {name}"),
            "Run so help to see every command and alias.",
        ));
    }
    if found.len() > 1 {
        let dir = &found[0];
        let global = &found[1];
        bail!(
            "{name} exists as a directory alias ({}) and a global alias ({}). Rename the global one with so alias rename --global {name} <new>, or remove the directory alias with so alias rm --dir {name}.",
            dir.path.display(),
            global.path.display()
        );
    }
    let record = &found[0];
    if record.integrity == Integrity::Invalid {
        bail!("alias {name} was modified and its signature no longer matches. Run so alias edit.");
    }
    if trust_of(store, record)? == Trust::None {
        let scope = record.scope.flag();
        bail!(
            "alias {name} by {} is not trusted. Run so alias trust {scope} {name}, or so alias trust {scope} {name} --author.",
            record.file.author.github
        );
    }
    Ok(RunPlan {
        body: record.file.body.clone(),
        args: args.to_vec(),
        depth,
    })
}

pub fn execute(plan: &RunPlan) -> Result<i32> {
    let status = Command::new("sh")
        .arg("-c")
        .arg(&plan.body)
        .arg("so-alias")
        .args(&plan.args)
        .env("SHELLOPS_ALIAS_DEPTH", (plan.depth + 1).to_string())
        .status()
        .context("could not run sh")?;
    Ok(status.code().unwrap_or(1))
}

pub fn command_args(plan: &RunPlan) -> Vec<String> {
    let mut args = vec![
        "sh".to_string(),
        "-c".to_string(),
        plan.body.clone(),
        "so-alias".to_string(),
    ];
    args.extend(plan.args.iter().cloned());
    args
}

pub fn trust_of(store: &Store, record: &Record) -> Result<Trust> {
    if record.integrity == Integrity::Invalid {
        return Ok(Trust::None);
    }
    if record.integrity == Integrity::Valid
        && let Some(identity) = identity::load(store)?
        && identity.fingerprint == record.file.author.fingerprint
        && identity.github == record.file.author.github
    {
        return Ok(Trust::OwnKey);
    }
    let trust = read_trust(store)?;
    if record.integrity == Integrity::Valid
        && trust.author.iter().any(|item| {
            item.github == record.file.author.github
                && item.fingerprint == record.file.author.fingerprint
        })
    {
        return Ok(Trust::Author);
    }
    if trust
        .hash
        .iter()
        .any(|item| item.sha256 == record.file.sha256)
    {
        return Ok(Trust::Hash);
    }
    Ok(Trust::None)
}

pub fn help_section(roots: &Roots, store: &Store) -> Result<String> {
    let records = list_records(roots)?;
    if records.is_empty() {
        return Ok(String::new());
    }
    let theme = Theme::stdout();
    let mut rows = Vec::new();
    for record in &records {
        rows.push((record, trust_of(store, record)?));
    }
    Ok(alias_lines(theme, &rows))
}

/// The aliases as a section in the style of the help screen: a pointer, the name, then its
/// scope and whether it runs.
pub fn alias_lines(theme: Theme, rows: &[(&Record, Trust)]) -> String {
    let icons = theme.icons();
    let cells: Vec<Vec<String>> = rows
        .iter()
        .map(|(record, _)| {
            vec![
                theme.accent(icons.pointer),
                theme.command(&record.file.name),
            ]
        })
        .collect();
    let grid = ui::Grid::fit(2, cells.iter().map(Vec::as_slice));
    let mut out = format!("{}\n", ui::section_line(theme, "Aliases"));
    for ((record, trust), cells) in rows.iter().zip(&cells) {
        let scope = format!("{} alias", record.scope.label());
        let about = match trust {
            Trust::None => format!(
                "{}{}{}",
                theme.dim(scope),
                theme.sep(),
                theme.caution("not trusted, will not run")
            ),
            _ => theme.dim(scope),
        };
        out.push_str(&grid.row(cells, &about));
    }
    out
}

pub fn print_record(store: &Store, record: &Record) -> Result<()> {
    let theme = Theme::stdout();
    let check = key_label(store, record);
    let trust = match trust_of(store, record)? {
        Trust::OwnKey => "yes, your key",
        Trust::Author => "yes, trusted author",
        Trust::Hash => "yes, trusted contents",
        Trust::None => "no",
    };
    println!(
        "{}",
        ui::panel(
            theme,
            vec![
                (
                    "name",
                    theme.cell(&record.file.name, Some(Color::Cyan), &[Attribute::Bold]),
                ),
                ("scope", theme.cell(record.scope.label(), None, &[])),
                ("path", theme.token_cell(&ui::tilde(&record.path))),
                ("author", theme.cell(&record.file.author.github, None, &[])),
                (
                    "fingerprint",
                    theme.cell(&record.file.author.fingerprint, None, &[]),
                ),
                ("github", theme.cell(check, None, &[])),
                (
                    "signature",
                    theme.cell(integrity_label(record.integrity), None, &[])
                ),
                ("trusted", theme.cell(trust, None, &[])),
                ("sha256", theme.cell(&record.file.sha256, None, &[])),
            ],
        )
    );
    ui::section("Body");
    println!("{}", record.file.body);
    Ok(())
}

fn key_label(store: &Store, record: &Record) -> String {
    match identity::key_status(
        store,
        &record.file.author.github,
        &record.file.author.public_key,
        &identity::keys_base(),
    ) {
        KeyCheck::Listed => "key is on this GitHub account".to_string(),
        KeyCheck::Missing => "key is not on this GitHub account".to_string(),
        KeyCheck::Unknown => "could not check GitHub".to_string(),
    }
}

fn integrity_label(integrity: Integrity) -> &'static str {
    match integrity {
        Integrity::Valid => "valid",
        Integrity::Unsigned => "absent",
        Integrity::Invalid => "invalid",
    }
}

fn write_signed(
    roots: &Roots,
    _store: &Store,
    signer: &Signer,
    scope: Scope,
    name: &str,
    body: &str,
    identity: &Identity,
) -> Result<Record> {
    let signature = identity::sign_body(signer, &identity.public_key, body)?;
    identity::verify_signature(&identity.public_key, body, &signature)?;
    let file = AliasFile {
        name: name.to_string(),
        sha256: identity::sha256_hex(body),
        body: body.to_string(),
        author: Author {
            github: identity.github.clone(),
            public_key: identity.public_key.clone(),
            fingerprint: identity.fingerprint.clone(),
        },
        signature: Some(Signature { sshsig: signature }),
    };
    let path = alias_path(roots, scope, name);
    write_file(&path, &file, scope)?;
    load_path(scope, path)
}

fn ensure_creatable(roots: &Roots, scope: Scope, name: &str) -> Result<()> {
    if path_of(roots, scope, name).is_file() {
        bail!(
            "alias {name} already exists. Remove it first with so alias rm {} {name}.",
            scope.flag()
        );
    }
    if scope == Scope::Dir && path_of(roots, Scope::Global, name).is_file() {
        bail!(
            "global alias {name} already exists. Remove it with so alias rm --global {name} before creating a directory alias."
        );
    }
    Ok(())
}

fn require_body(body: &str) -> Result<String> {
    if body.trim().is_empty() {
        bail!("alias body is empty");
    }
    Ok(body.to_string())
}

fn require_one(roots: &Roots, name: &str, scope: Option<Scope>) -> Result<Record> {
    let found = find(roots, name, scope)?;
    match found.len() {
        0 => bail!("alias {name} was not found"),
        1 => Ok(found.into_iter().next().expect("one alias")),
        _ => bail!("{name} exists in both scopes. Pass --global or --dir."),
    }
}

fn find(roots: &Roots, name: &str, scope: Option<Scope>) -> Result<Vec<Record>> {
    let mut found = Vec::new();
    let scopes = match scope {
        Some(scope) => vec![scope],
        None => vec![Scope::Dir, Scope::Global],
    };
    for scope in scopes {
        let path = path_of(roots, scope, name);
        if path.is_file() {
            found.push(load_path(scope, path)?);
        }
    }
    Ok(found)
}

fn read_dir(roots: &Roots, scope: Scope) -> Result<Vec<Record>> {
    let dir = match scope {
        Scope::Global => roots.home.join("aliases"),
        Scope::Dir => roots.dir.join(".shellops").join("aliases"),
    };
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut names: Vec<PathBuf> = fs::read_dir(&dir)
        .with_context(|| format!("could not read {}", dir.display()))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("toml"))
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|path| load_path(scope, path))
        .collect()
}

fn load_path(scope: Scope, path: PathBuf) -> Result<Record> {
    let text =
        fs::read_to_string(&path).with_context(|| format!("could not read {}", path.display()))?;
    let file: AliasFile =
        toml::from_str(&text).with_context(|| format!("could not parse {}", path.display()))?;
    let integrity = integrity_of(&file);
    Ok(Record {
        scope,
        path,
        file,
        integrity,
    })
}

fn integrity_of(file: &AliasFile) -> Integrity {
    if identity::sha256_hex(&file.body) != file.sha256 {
        return Integrity::Invalid;
    }
    match &file.signature {
        None => Integrity::Unsigned,
        Some(signature) => {
            if identity::verify_signature(&file.author.public_key, &file.body, &signature.sshsig)
                .is_ok()
                && identity::fingerprint_of(&file.author.public_key)
                    .ok()
                    .as_deref()
                    == Some(file.author.fingerprint.as_str())
            {
                Integrity::Valid
            } else {
                Integrity::Invalid
            }
        }
    }
}

fn write_file(path: &Path, file: &AliasFile, scope: Scope) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("could not create {}", parent.display()))?;
    }
    let text = toml::to_string_pretty(file).context("could not encode the alias")?;
    let temporary = path.with_extension("toml.tmp");
    fs::write(&temporary, text)
        .with_context(|| format!("could not write {}", temporary.display()))?;
    set_mode(&temporary, mode_for(scope))?;
    fs::rename(&temporary, path).with_context(|| format!("could not write {}", path.display()))?;
    Ok(())
}

fn mode_for(scope: Scope) -> u32 {
    match scope {
        Scope::Global => 0o600,
        Scope::Dir => 0o644,
    }
}

fn set_mode(path: &Path, mode: u32) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    }
    let _ = mode;
    Ok(())
}

fn alias_path(roots: &Roots, scope: Scope, name: &str) -> PathBuf {
    path_of(roots, scope, name)
}

fn path_of(roots: &Roots, scope: Scope, name: &str) -> PathBuf {
    match scope {
        Scope::Global => roots.home.join("aliases").join(format!("{name}.toml")),
        Scope::Dir => roots
            .dir
            .join(".shellops")
            .join("aliases")
            .join(format!("{name}.toml")),
    }
}

fn read_trust(store: &Store) -> Result<TrustFile> {
    let path = trust_path(store);
    if !path.is_file() {
        return Ok(TrustFile::default());
    }
    let text =
        fs::read_to_string(&path).with_context(|| format!("could not read {}", path.display()))?;
    if text.trim().is_empty() {
        return Ok(TrustFile::default());
    }
    toml::from_str(&text).with_context(|| format!("could not parse {}", path.display()))
}

fn write_trust(store: &Store, trust: &TrustFile) -> Result<()> {
    let path = trust_path(store);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
        }
    }
    let text = toml::to_string_pretty(trust).context("could not encode trust")?;
    fs::write(&path, text).with_context(|| format!("could not write {}", path.display()))?;
    set_mode(&path, 0o600)?;
    Ok(())
}

fn trust_path(store: &Store) -> PathBuf {
    store.dir.join("trust.toml")
}

fn scope_order(scope: Scope) -> u8 {
    match scope {
        Scope::Dir => 0,
        Scope::Global => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Identity;
    use rand_core::OsRng;
    use ssh_key::{Algorithm, LineEnding, PrivateKey};

    fn record(name: &str, scope: Scope) -> Record {
        Record {
            scope,
            path: PathBuf::from(format!("/tmp/{name}.toml")),
            file: AliasFile {
                name: name.to_string(),
                body: "echo hi".to_string(),
                sha256: "0".repeat(64),
                author: Author {
                    github: "octocat".to_string(),
                    public_key: String::new(),
                    fingerprint: String::new(),
                },
                signature: None,
            },
            integrity: Integrity::Unsigned,
        }
    }

    #[test]
    fn alias_section_matches_the_help_screen() {
        let deploy = record("deploy", Scope::Global);
        let seed = record("seed-the-database", Scope::Dir);
        let text = alias_lines(
            ui::Theme::plain(),
            &[(&deploy, Trust::OwnKey), (&seed, Trust::None)],
        );
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "==> Aliases");
        assert_eq!(lines[1], "  ▸  deploy             global alias");
        assert_eq!(
            lines[2],
            "  ▸  seed-the-database  dir alias · not trusted, will not run"
        );
        let colored = alias_lines(ui::Theme::colored(), &[(&deploy, Trust::Hash)]);
        assert_eq!(
            ui::layout::strip_ansi(&colored),
            alias_lines(ui::Theme::plain(), &[(&deploy, Trust::Hash)])
        );
    }

    struct World {
        roots: Roots,
        store: Store,
        signer: Signer,
        _dir: tempfile::TempDir,
    }

    impl World {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let home = dir.path().join("home");
            let project = dir.path().join("project");
            fs::create_dir_all(&home).unwrap();
            fs::create_dir_all(project.join("sub")).unwrap();
            let private = PrivateKey::random(&mut OsRng, Algorithm::Ed25519).unwrap();
            let pem = private.to_openssh(LineEnding::LF).unwrap().to_string();
            let public = private.public_key().to_openssh().unwrap();
            let store = Store::at(&home);
            let identity = Identity {
                github: "alice".to_string(),
                public_key: public.trim().to_string(),
                fingerprint: identity::fingerprint_of(public.trim()).unwrap(),
            };
            identity::save(&store, &identity).unwrap();
            Self {
                roots: Roots { home, dir: project },
                store,
                signer: Signer {
                    private_pem: Some(pem),
                    keygen: PathBuf::from("ssh-keygen"),
                },
                _dir: dir,
            }
        }
    }

    #[test]
    fn reserved_names_and_conflicts() {
        let world = World::new();
        assert!(validate_name("c").is_err());
        assert!(validate_name("Fix").is_err());
        create(
            &world.roots,
            &world.store,
            &world.signer,
            Scope::Global,
            "fixall",
            "so csf",
        )
        .unwrap();
        let err = create(
            &world.roots,
            &world.store,
            &world.signer,
            Scope::Dir,
            "fixall",
            "so csf",
        )
        .unwrap_err();
        assert!(err.to_string().contains("global alias"));
        create(
            &world.roots,
            &world.store,
            &world.signer,
            Scope::Global,
            "lint",
            "so phpstan",
        )
        .unwrap();
        let err = create(
            &world.roots,
            &world.store,
            &world.signer,
            Scope::Global,
            "lint",
            "so phan",
        )
        .unwrap_err();
        assert!(err.to_string().contains("Remove it first"));
    }

    #[test]
    fn own_alias_runs_and_a_forged_author_does_not() {
        let world = World::new();
        create(
            &world.roots,
            &world.store,
            &world.signer,
            Scope::Dir,
            "fixall",
            "so csf && so phpstan",
        )
        .unwrap();
        let plan = plan_run(
            &world.roots,
            &world.store,
            "fixall",
            &["--dry-run".into()],
            0,
        )
        .unwrap();
        assert_eq!(
            command_args(&plan),
            vec!["sh", "-c", "so csf && so phpstan", "so-alias", "--dry-run",]
        );
        let other = PrivateKey::random(&mut OsRng, Algorithm::Ed25519).unwrap();
        let other_pem = other.to_openssh(LineEnding::LF).unwrap().to_string();
        let forged_sig = identity::sign_with_private(&other_pem, "so csf && so phpstan").unwrap();
        let mut file = view_records(&world.roots, "fixall", Some(Scope::Dir))
            .unwrap()
            .remove(0)
            .file;
        file.author.public_key = other.public_key().to_openssh().unwrap().trim().to_string();
        file.author.fingerprint = identity::fingerprint_of(&file.author.public_key).unwrap();
        file.signature = Some(Signature { sshsig: forged_sig });
        write_file(
            &path_of(&world.roots, Scope::Dir, "fixall"),
            &file,
            Scope::Dir,
        )
        .unwrap();
        trust_author(&world.roots, &world.store, "fixall", Some(Scope::Dir)).unwrap_err();
        let record = view_records(&world.roots, "fixall", Some(Scope::Dir))
            .unwrap()
            .remove(0);
        assert_eq!(trust_of(&world.store, &record).unwrap(), Trust::None);
        let err = plan_run(&world.roots, &world.store, "fixall", &[], 0).unwrap_err();
        assert!(err.to_string().contains("not trusted"));
    }

    #[test]
    fn tampered_body_never_runs_even_when_the_new_hash_is_trusted() {
        let world = World::new();
        create(
            &world.roots,
            &world.store,
            &world.signer,
            Scope::Global,
            "fixall",
            "echo ok",
        )
        .unwrap();
        let path = path_of(&world.roots, Scope::Global, "fixall");
        let mut file: AliasFile = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        file.body = "echo pwned".to_string();
        file.sha256 = identity::sha256_hex(&file.body);
        fs::write(&path, toml::to_string_pretty(&file).unwrap()).unwrap();
        trust_hash(&world.roots, &world.store, "fixall", Some(Scope::Global)).unwrap_err();
        let err = plan_run(&world.roots, &world.store, "fixall", &[], 0).unwrap_err();
        assert!(err.to_string().contains("signature"));
    }

    #[test]
    fn hash_trust_runs_an_unsigned_alias_and_author_trust_runs_a_new_body() {
        let world = World::new();
        let bob = PrivateKey::random(&mut OsRng, Algorithm::Ed25519).unwrap();
        let bob_pem = bob.to_openssh(LineEnding::LF).unwrap().to_string();
        let public = bob.public_key().to_openssh().unwrap().trim().to_string();
        let body = "echo from-bob";
        let signature = identity::sign_with_private(&bob_pem, body).unwrap();
        let file = AliasFile {
            name: "lint".to_string(),
            body: body.to_string(),
            sha256: identity::sha256_hex(body),
            author: Author {
                github: "bob".to_string(),
                fingerprint: identity::fingerprint_of(&public).unwrap(),
                public_key: public,
            },
            signature: Some(Signature { sshsig: signature }),
        };
        write_file(
            &path_of(&world.roots, Scope::Dir, "lint"),
            &file,
            Scope::Dir,
        )
        .unwrap();
        assert!(plan_run(&world.roots, &world.store, "lint", &[], 0).is_err());
        let cache = world.store.dir.join("github-keys");
        fs::create_dir_all(&cache).unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        fs::write(
            cache.join("bob.json"),
            format!(
                r#"{{"checked_at":{now},"keys":[{}]}}"#,
                serde_json::to_string(&file.author.public_key).unwrap()
            ),
        )
        .unwrap();
        trust_author(&world.roots, &world.store, "lint", Some(Scope::Dir)).unwrap();
        assert!(plan_run(&world.roots, &world.store, "lint", &[], 0).is_ok());
        let edited = "echo from-bob-2";
        let edited_sig = identity::sign_with_private(&bob_pem, edited).unwrap();
        let mut next = file.clone();
        next.body = edited.to_string();
        next.sha256 = identity::sha256_hex(edited);
        next.signature = Some(Signature { sshsig: edited_sig });
        write_file(
            &path_of(&world.roots, Scope::Dir, "lint"),
            &next,
            Scope::Dir,
        )
        .unwrap();
        assert!(plan_run(&world.roots, &world.store, "lint", &[], 0).is_ok());

        let unsigned = AliasFile {
            name: "note".to_string(),
            body: "echo unsigned".to_string(),
            sha256: identity::sha256_hex("echo unsigned"),
            author: file.author.clone(),
            signature: None,
        };
        write_file(
            &path_of(&world.roots, Scope::Global, "note"),
            &unsigned,
            Scope::Global,
        )
        .unwrap();
        assert!(plan_run(&world.roots, &world.store, "note", &[], 0).is_err());
        trust_hash(&world.roots, &world.store, "note", Some(Scope::Global)).unwrap();
        assert!(plan_run(&world.roots, &world.store, "note", &[], 0).is_ok());
        untrust_hash(&world.roots, &world.store, "note", Some(Scope::Global)).unwrap();
        assert!(plan_run(&world.roots, &world.store, "note", &[], 0).is_err());
    }

    #[test]
    fn both_scopes_block_the_run_and_depth_stops_recursion() {
        let world = World::new();
        create(
            &world.roots,
            &world.store,
            &world.signer,
            Scope::Dir,
            "fixall",
            "echo dir",
        )
        .unwrap();
        create(
            &world.roots,
            &world.store,
            &world.signer,
            Scope::Global,
            "fixall",
            "echo global",
        )
        .unwrap();
        let err = plan_run(&world.roots, &world.store, "fixall", &[], 0).unwrap_err();
        assert!(err.to_string().contains("rename --global"));
        remove(&world.roots, Scope::Dir, "fixall").unwrap();
        assert!(plan_run(&world.roots, &world.store, "fixall", &[], MAX_DEPTH).is_err());
    }

    #[test]
    fn rename_keeps_the_signature_and_edit_changes_the_author() {
        let world = World::new();
        create(
            &world.roots,
            &world.store,
            &world.signer,
            Scope::Dir,
            "fixall",
            "echo one",
        )
        .unwrap();
        rename(&world.roots, Scope::Dir, "fixall", "lint").unwrap();
        assert!(!path_of(&world.roots, Scope::Dir, "fixall").exists());
        let renamed = view_records(&world.roots, "lint", None).unwrap().remove(0);
        assert_eq!(renamed.integrity, Integrity::Valid);
        assert_eq!(renamed.file.author.github, "alice");
        let bob_key = PrivateKey::random(&mut OsRng, Algorithm::Ed25519).unwrap();
        let bob = Identity {
            github: "bob".to_string(),
            public_key: bob_key
                .public_key()
                .to_openssh()
                .unwrap()
                .trim()
                .to_string(),
            fingerprint: identity::fingerprint_of(
                bob_key.public_key().to_openssh().unwrap().trim(),
            )
            .unwrap(),
        };
        identity::save(&world.store, &bob).unwrap();
        let bob_signer = Signer {
            private_pem: Some(bob_key.to_openssh(LineEnding::LF).unwrap().to_string()),
            keygen: PathBuf::from("ssh-keygen"),
        };
        edit(
            &world.roots,
            &world.store,
            &bob_signer,
            Scope::Dir,
            "lint",
            "echo two",
        )
        .unwrap();
        let edited = view_records(&world.roots, "lint", None).unwrap().remove(0);
        assert_eq!(edited.file.author.github, "bob");
        assert_eq!(edited.file.body, "echo two");
        assert_eq!(edited.integrity, Integrity::Valid);
        assert_eq!(trust_of(&world.store, &edited).unwrap(), Trust::OwnKey);
    }
}
