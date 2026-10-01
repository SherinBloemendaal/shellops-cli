use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ssh_key::{HashAlg, LineEnding, PrivateKey, PublicKey, SshSig};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::config::Store;

pub const NAMESPACE: &str = "shellops-alias";
const KEY_CACHE_SECS: i64 = 24 * 60 * 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub github: String,
    pub public_key: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyCheck {
    Listed,
    Missing,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct Signer {
    pub private_pem: Option<String>,
    pub keygen: PathBuf,
}

impl Signer {
    pub fn system() -> Self {
        let private_pem = std::env::var("SHELLOPS_TEST_SIGNING_KEY")
            .ok()
            .and_then(|path| fs::read_to_string(path).ok());
        let keygen = std::env::var("SHELLOPS_SSH_KEYGEN")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("ssh-keygen"));
        Self {
            private_pem,
            keygen,
        }
    }
}

pub fn load(store: &Store) -> Result<Option<Identity>> {
    let github = plain(store, "identity.github")?;
    let public_key = plain(store, "identity.public_key")?;
    let fingerprint = plain(store, "identity.fingerprint")?;
    match (github, public_key, fingerprint) {
        (None, None, None) => Ok(None),
        (Some(github), Some(public_key), Some(fingerprint)) => Ok(Some(Identity {
            github,
            public_key,
            fingerprint,
        })),
        _ => bail!("signing identity is incomplete. Run so init identity."),
    }
}

pub fn require(store: &Store) -> Result<Identity> {
    load(store)?.context("no signing identity. Run so init identity.")
}

pub fn save(store: &Store, identity: &Identity) -> Result<()> {
    store.set("identity.github", &identity.github, false, None)?;
    store.set("identity.public_key", &identity.public_key, false, None)?;
    store.set("identity.fingerprint", &identity.fingerprint, false, None)?;
    Ok(())
}

pub fn accept_identity(github: &str, public_key: &str, github_keys: &[String]) -> Result<Identity> {
    validate_login(github)?;
    let fingerprint = fingerprint_of(public_key)?;
    if !listed(public_key, github_keys)? {
        bail!("{github} has no GitHub SSH key matching {fingerprint}");
    }
    Ok(Identity {
        github: github.to_string(),
        public_key: normalize_key(public_key)?,
        fingerprint,
    })
}

pub fn validate_login(github: &str) -> Result<()> {
    let mut chars = github.chars();
    let Some(first) = chars.next() else {
        bail!("GitHub login is empty");
    };
    if !first.is_ascii_alphanumeric()
        || !github
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
    {
        bail!("invalid GitHub login {github}");
    }
    Ok(())
}

pub fn fingerprint_of(openssh: &str) -> Result<String> {
    let key = parse_key(openssh)?;
    Ok(key.fingerprint(HashAlg::Sha256).to_string())
}

pub fn normalize_key(openssh: &str) -> Result<String> {
    let key = parse_key(openssh)?;
    let mut line = key.to_openssh()?;
    if !line.ends_with('\n') {
        line.push('\n');
    }
    Ok(line.trim_end().to_string())
}

pub fn listed(public_key: &str, github_keys: &[String]) -> Result<bool> {
    let want = fingerprint_of(public_key)?;
    for line in github_keys {
        if line.trim().is_empty() || line.trim().starts_with('#') {
            continue;
        }
        if fingerprint_of(line).ok().as_deref() == Some(want.as_str()) {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn sign_body(signer: &Signer, public_key: &str, body: &str) -> Result<String> {
    if let Some(pem) = &signer.private_pem {
        return sign_with_private(pem, body);
    }
    sign_with_keygen(&signer.keygen, public_key, body)
}

pub fn sign_with_private(pem: &str, body: &str) -> Result<String> {
    let key = PrivateKey::from_openssh(pem).context("could not read the test signing key")?;
    let sig = SshSig::sign(&key, NAMESPACE, HashAlg::Sha512, body.as_bytes())
        .context("could not sign the alias")?;
    sig.to_pem(LineEnding::LF)
        .context("could not encode the alias signature")
}

pub fn verify_signature(public_key: &str, body: &str, armored: &str) -> Result<()> {
    let key = parse_key(public_key)?;
    let sig = SshSig::from_pem(armored).context("alias signature is not valid SSHSIG")?;
    key.verify(NAMESPACE, body.as_bytes(), &sig)
        .context("alias signature does not match the body")?;
    Ok(())
}

pub fn discover_keys() -> Result<Vec<String>> {
    let mut keys = Vec::new();
    if let Ok(output) = Command::new("ssh-add").arg("-L").output()
        && output.status.success()
    {
        push_key_lines(&String::from_utf8_lossy(&output.stdout), &mut keys);
    }
    if let Ok(home) = crate::project::home_dir() {
        let ssh = home.join(".ssh");
        if let Ok(entries) = fs::read_dir(&ssh) {
            let mut paths: Vec<PathBuf> = entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("pub"))
                .collect();
            paths.sort();
            for path in paths {
                if let Ok(text) = fs::read_to_string(&path) {
                    push_key_lines(&text, &mut keys);
                }
            }
        }
    }
    if keys.is_empty() {
        bail!("no SSH public keys found. Run ssh-add, or create a key with ssh-keygen.");
    }
    Ok(keys)
}

pub fn gh_login() -> Option<String> {
    let output = Command::new("gh")
        .args(["api", "user", "--jq", ".login"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let login = String::from_utf8_lossy(&output.stdout).trim().to_string();
    validate_login(&login).ok()?;
    Some(login)
}

pub fn key_status(store: &Store, github: &str, public_key: &str, keys_base: &str) -> KeyCheck {
    match github_keys(store, github, keys_base, false) {
        Ok(keys) => match listed(public_key, &keys) {
            Ok(true) => KeyCheck::Listed,
            Ok(false) => KeyCheck::Missing,
            Err(_) => KeyCheck::Unknown,
        },
        Err(_) => KeyCheck::Unknown,
    }
}

pub fn github_keys(
    store: &Store,
    github: &str,
    keys_base: &str,
    refresh: bool,
) -> Result<Vec<String>> {
    validate_login(github)?;
    let path = cache_path(store, github)?;
    let now = unix_now();
    if !refresh
        && let Some(cache) = read_cache(&path)
        && now.saturating_sub(cache.checked_at) < KEY_CACHE_SECS
    {
        return Ok(cache.keys);
    }
    match fetch_keys(keys_base, github) {
        Ok(keys) => {
            let _ = write_cache(&path, now, &keys);
            Ok(keys)
        }
        Err(err) => {
            if let Some(cache) = read_cache(&path) {
                return Ok(cache.keys);
            }
            Err(err)
        }
    }
}

pub fn keys_base() -> String {
    std::env::var("SHELLOPS_GITHUB_KEYS_BASE").unwrap_or_else(|_| "https://github.com".to_string())
}

fn fetch_keys(keys_base: &str, github: &str) -> Result<Vec<String>> {
    let url = format!("{}/{github}.keys", keys_base.trim_end_matches('/'));
    let response = agent()
        .get(&url)
        .set("User-Agent", concat!("so/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|err| anyhow::anyhow!("could not read GitHub SSH keys for {github}: {err}"))?;
    let text = response
        .into_string()
        .context("could not read GitHub SSH keys")?;
    let mut keys = Vec::new();
    push_key_lines(&text, &mut keys);
    Ok(keys)
}

fn sign_with_keygen(keygen: &Path, public_key: &str, body: &str) -> Result<String> {
    let dir = tempfile::tempdir().context("could not create a signing directory")?;
    let pub_path = dir.path().join("id.pub");
    let body_path = dir.path().join("body");
    fs::write(&pub_path, public_key).context("could not write the public key")?;
    fs::write(&body_path, body).context("could not write the alias body")?;
    let status = Command::new(keygen)
        .args(["-Y", "sign", "-n", NAMESPACE, "-f"])
        .arg(&pub_path)
        .arg(&body_path)
        .status()
        .with_context(|| format!("could not run {}", keygen.display()))?;
    if !status.success() {
        bail!("ssh-keygen could not sign the alias. Is that key loaded in ssh-agent?");
    }
    let sig_path = dir.path().join("body.sig");
    fs::read_to_string(&sig_path).context("ssh-keygen did not write a signature")
}

fn parse_key(openssh: &str) -> Result<PublicKey> {
    PublicKey::from_openssh(openssh.trim()).context("invalid SSH public key")
}

fn push_key_lines(text: &str, keys: &mut Vec<String>) {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Ok(normalized) = normalize_key(line)
            && !keys.iter().any(|existing| existing == &normalized)
        {
            keys.push(normalized);
        }
    }
}

fn plain(store: &Store, key: &str) -> Result<Option<String>> {
    Ok(store.lookup(key, None)?.map(|hit| hit.value))
}

fn cache_path(store: &Store, github: &str) -> Result<PathBuf> {
    validate_login(github)?;
    Ok(store.dir.join("github-keys").join(format!("{github}.json")))
}

#[derive(Debug, Serialize, Deserialize)]
struct KeyCache {
    checked_at: i64,
    keys: Vec<String>,
}

fn read_cache(path: &Path) -> Option<KeyCache> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_cache(path: &Path, now: i64, keys: &[String]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let body = serde_json::to_string(&KeyCache {
        checked_at: now,
        keys: keys.to_vec(),
    })?;
    fs::write(path, body)?;
    Ok(())
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(20))
        .redirects(8)
        .build()
}

pub fn sha256_hex(body: &str) -> String {
    let digest = Sha256::digest(body.as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push(char::from(b"0123456789abcdef"[usize::from(byte >> 4)]));
        out.push(char::from(b"0123456789abcdef"[usize::from(byte & 0xf)]));
    }
    out
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_core::OsRng;
    use ssh_key::Algorithm;

    fn keypair() -> (PrivateKey, String) {
        let private = PrivateKey::random(&mut OsRng, Algorithm::Ed25519).unwrap();
        let public = private.public_key().to_openssh().unwrap();
        (private, public.trim().to_string())
    }

    #[test]
    fn signature_roundtrip_and_reject_a_changed_body() {
        let (private, public) = keypair();
        let pem = private.to_openssh(LineEnding::LF).unwrap().to_string();
        let signature = sign_with_private(&pem, "so csf").unwrap();
        verify_signature(&public, "so csf", &signature).unwrap();
        let err = verify_signature(&public, "so phpstan", &signature).unwrap_err();
        assert!(err.to_string().contains("does not match"));
    }

    #[test]
    fn github_login_must_match_a_listed_key() {
        let (_private, public) = keypair();
        let (_other, other) = keypair();
        let identity = accept_identity("Sherin", &public, std::slice::from_ref(&public)).unwrap();
        assert_eq!(identity.github, "Sherin");
        assert!(accept_identity("Sherin", &public, &[other]).is_err());
        assert!(accept_identity("-nope", &public, std::slice::from_ref(&public)).is_err());
    }

    #[test]
    fn keygen_is_invoked_with_the_alias_namespace() {
        let dir = tempfile::tempdir().unwrap();
        let keygen = dir.path().join("ssh-keygen");
        fs::write(
            &keygen,
            "#!/bin/sh\nlog=$(dirname \"$0\")/log\nprintf '%s\\n' \"$@\" >> \"$log\"\nfile=\nfor arg in \"$@\"; do file=$arg; done\nprintf 'signed\\n' > \"$file.sig\"\nexit 0\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&keygen, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let signature = sign_with_keygen(&keygen, "ssh-ed25519 AAAA test", "body").unwrap();
        assert_eq!(signature.trim(), "signed");
        let log = fs::read_to_string(dir.path().join("log")).unwrap();
        assert!(log.contains("-Y\nsign\n-n\nshellops-alias\n"), "{log}");
    }

    #[test]
    fn fresh_key_cache_skips_the_network() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::at(dir.path());
        let cache = dir.path().join("github-keys");
        fs::create_dir_all(&cache).unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        fs::write(
            cache.join("octocat.json"),
            format!(r#"{{"checked_at":{now},"keys":["ssh-ed25519 AAAA cached"]}}"#),
        )
        .unwrap();
        let keys = github_keys(&store, "octocat", "http://127.0.0.1:1", false).unwrap();
        assert_eq!(keys, vec!["ssh-ed25519 AAAA cached".to_string()]);
    }
}
