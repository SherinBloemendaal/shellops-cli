//! Drives `so` inside a real pseudo-terminal (via `script`) to check what a person sees when a
//! prompt is interrupted with Ctrl-C. Only safe, local steps run: `so init` with a scratch
//! HOME stops at its first menu before anything is saved or any network call is made.

#![cfg(unix)]

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const HIDE_CURSOR: &str = "\u{1b}[?25l";
const SHOW_CURSOR: &str = "\u{1b}[?25h";

fn so_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_so"))
}

fn has_script() -> bool {
    Command::new("script")
        .arg("-q")
        .arg("/dev/null")
        .arg("true")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

/// Runs `shell` (a `sh -c` script) inside a pseudo-terminal 80 columns wide.
fn in_pty(shell: &str, home: &std::path::Path) -> Child {
    let shell = format!("stty cols 80 rows 40; {shell}");
    let mut command = Command::new("script");
    if cfg!(target_os = "macos") {
        command.args(["-q", "/dev/null", "sh", "-c", &shell]);
    } else {
        command.args(["-qec", &format!("sh -c '{shell}'"), "/dev/null"]);
    }
    command
        .env("HOME", home)
        .env("TERM", "xterm-256color")
        .env("LANG", "en_US.UTF-8")
        .env("SHELLOPS_NO_UPDATE_CHECK", "1")
        .env("SHELLOPS_NO_SETUP_HINT", "1")
        .env_remove("NO_COLOR")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("script runs")
}

fn collect(child: &mut Child) -> Arc<Mutex<String>> {
    let out = Arc::new(Mutex::new(String::new()));
    let mut stdout = child.stdout.take().unwrap();
    let sink = Arc::clone(&out);
    std::thread::spawn(move || {
        let mut buf = [0_u8; 4096];
        while let Ok(read) = stdout.read(&mut buf) {
            if read == 0 {
                break;
            }
            sink.lock()
                .unwrap()
                .push_str(&String::from_utf8_lossy(&buf[..read]));
        }
    });
    out
}

fn wait_for(out: &Arc<Mutex<String>>, needle: &str) -> bool {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(15) {
        if out.lock().unwrap().contains(needle) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

fn finish(mut child: Child) {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(15) {
        if child.try_wait().unwrap().is_some() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    panic!("so did not exit after Ctrl-C");
}

#[test]
fn ctrl_c_in_a_menu_gives_the_cursor_back() {
    if !has_script() {
        eprintln!("skipped: no script(1)");
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let so = so_bin();
    let mut child = in_pty(
        &format!("trap true INT; {} init; echo; echo after-so", so.display()),
        home.path(),
    );
    let out = collect(&mut child);
    assert!(
        wait_for(&out, "What do you want to do?"),
        "menu never appeared: {:?}",
        out.lock().unwrap()
    );
    assert!(wait_for(&out, HIDE_CURSOR), "the menu hides the cursor");
    std::thread::sleep(Duration::from_millis(200));
    child.stdin.as_mut().unwrap().write_all(b"\x03").unwrap();
    assert!(wait_for(&out, "after-so"), "{:?}", out.lock().unwrap());
    finish(child);
    let text = out.lock().unwrap().clone();
    let hidden = text.rfind(HIDE_CURSOR).unwrap();
    assert!(
        text[hidden..].contains(SHOW_CURSOR),
        "cursor left hidden after Ctrl-C: {:?}",
        &text[hidden..]
    );
    assert!(
        !home.path().join(".shellops").join("config.toml").exists(),
        "nothing was saved"
    );
}

#[test]
fn ctrl_c_in_a_password_prompt_turns_echo_back_on() {
    if !has_script() {
        eprintln!("skipped: no script(1)");
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let so = so_bin();
    let mut child = in_pty(
        &format!(
            "trap true INT; {} init openrouter; echo; echo echo-state:; stty -a",
            so.display()
        ),
        home.path(),
    );
    let out = collect(&mut child);
    assert!(wait_for(&out, "What do you want to do?"));
    std::thread::sleep(Duration::from_millis(200));
    child.stdin.as_mut().unwrap().write_all(b"\r").unwrap();
    assert!(
        wait_for(&out, "OpenRouter API key"),
        "{:?}",
        out.lock().unwrap()
    );
    std::thread::sleep(Duration::from_millis(200));
    child.stdin.as_mut().unwrap().write_all(b"\x03").unwrap();
    assert!(wait_for(&out, "echo-state:"), "{:?}", out.lock().unwrap());
    std::thread::sleep(Duration::from_millis(300));
    finish(child);
    let text = out.lock().unwrap().clone();
    let state = &text[text.rfind("echo-state:").unwrap()..];
    let words: Vec<&str> = state.split_whitespace().collect();
    assert!(
        words.contains(&"echo") && !words.contains(&"-echo"),
        "terminal echo left off after Ctrl-C: {state}"
    );
    assert!(
        !home.path().join(".shellops").join("secrets.toml").exists(),
        "nothing was saved"
    );
}
