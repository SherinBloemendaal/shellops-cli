use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn so_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_so"))
}

#[test]
fn init_without_a_terminal_points_at_config_set() {
    let home = tempfile::tempdir().unwrap();
    let output = Command::new(so_bin())
        .arg("init")
        .env("HOME", home.path())
        .env("SHELLOPS_NO_UPDATE_CHECK", "1")
        .env("SHELLOPS_NO_SETUP_HINT", "1")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!output.status.success());
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(err.contains("so config set"), "{err}");
}

#[test]
fn unknown_command_is_not_an_alias() {
    let home = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let output = Command::new(so_bin())
        .arg("not-a-command")
        .current_dir(root.path())
        .env("HOME", home.path())
        .env("SHELLOPS_NO_UPDATE_CHECK", "1")
        .env("SHELLOPS_NO_SETUP_HINT", "1")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(err.contains("unknown command"), "{err}");
}

#[test]
fn directory_alias_is_written_at_the_git_root() {
    let home = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    assert!(
        Command::new("git")
            .args(["init"])
            .current_dir(root.path())
            .status()
            .unwrap()
            .success()
    );
    let private =
        ssh_key::PrivateKey::random(&mut rand_core::OsRng, ssh_key::Algorithm::Ed25519).unwrap();
    let pem = private
        .to_openssh(ssh_key::LineEnding::LF)
        .unwrap()
        .to_string();
    let key_path = home.path().join("key");
    fs::write(&key_path, pem).unwrap();
    let public = private.public_key().to_openssh().unwrap();
    let fingerprint = private.public_key().fingerprint(ssh_key::HashAlg::Sha256);
    fs::create_dir_all(home.path().join(".shellops")).unwrap();
    fs::write(
        home.path().join(".shellops").join("config.toml"),
        format!(
            "[identity]\ngithub = \"alice\"\npublic_key = \"{}\"\nfingerprint = \"{fingerprint}\"\n",
            public.trim().replace('"', "\\\"")
        ),
    )
    .unwrap();
    let sub = root.path().join("app");
    fs::create_dir_all(&sub).unwrap();
    let output = Command::new(so_bin())
        .args(["alias", "create", "--dir", "fixall", "echo ok"])
        .current_dir(&sub)
        .env("HOME", home.path())
        .env("SHELLOPS_TEST_SIGNING_KEY", &key_path)
        .env("SHELLOPS_NO_UPDATE_CHECK", "1")
        .env("SHELLOPS_NO_SETUP_HINT", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let alias = root
        .path()
        .join(".shellops")
        .join("aliases")
        .join("fixall.toml");
    assert!(alias.is_file(), "missing {}", alias.display());
    assert!(!sub.join(".shellops").exists());
}
