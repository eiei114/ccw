//! Finding and running the `claude` CLI.

use std::env;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

pub type Env = Vec<(String, PathBuf)>;

const YOLO: &str = "--dangerously-skip-permissions";
const YOLO_ARGS: [&str; 2] = ["--allow-dangerously-skip-permissions", YOLO];

static SAFE_FLAG: AtomicBool = AtomicBool::new(false);

/// `--safe` / `--no-yolo` on the command line.
pub fn set_safe() {
    SAFE_FLAG.store(true, Ordering::Relaxed);
}

/// True when launches should not add --dangerously-skip-permissions.
pub fn is_safe() -> bool {
    if SAFE_FLAG.load(Ordering::Relaxed) {
        return true;
    }
    ["CCW_SAFE", "CCX_SAFE"]
        .iter()
        .any(|k| matches!(env::var(k).as_deref(), Ok("1") | Ok("true")))
}

/// Add the skip-permissions flags unless safe mode is on or the caller already passed them.
pub fn with_yolo(args: &[String]) -> Vec<String> {
    with_yolo_inner(args, is_safe())
}

fn with_yolo_inner(args: &[String], safe: bool) -> Vec<String> {
    let rest = match args.first() {
        Some(a) if a == "--" => &args[1..],
        _ => args,
    };
    let dropped: Vec<String> = rest.iter().filter(|a| *a != "--safe").cloned().collect();
    let wants_safe = dropped.len() != rest.len() || safe;
    if wants_safe || dropped.iter().any(|a| a == YOLO) {
        return dropped;
    }
    YOLO_ARGS
        .iter()
        .map(|s| s.to_string())
        .chain(dropped)
        .collect()
}

fn find_claude() -> PathBuf {
    let exts: &[&str] = if cfg!(windows) {
        &[".exe", ".cmd", ".bat", ""]
    } else {
        &[""]
    };
    if let Some(path) = env::var_os("PATH") {
        for dir in env::split_paths(&path) {
            if dir.as_os_str().is_empty() {
                continue;
            }
            for ext in exts {
                let candidate = dir.join(format!("claude{ext}"));
                if candidate.is_file() {
                    return candidate;
                }
            }
        }
    }
    PathBuf::from("claude")
}

pub fn claude_path() -> &'static Path {
    static CLAUDE: OnceLock<PathBuf> = OnceLock::new();
    CLAUDE.get_or_init(find_claude)
}

fn command(args: &[String], env: &Env) -> Command {
    // std quotes arguments for .cmd/.bat files itself (and refuses unsafe ones).
    let mut cmd = Command::new(claude_path());
    cmd.args(args);
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd
}

/// Keep ccw alive on Ctrl-C while claude owns the terminal; claude handles the key itself.
fn ignore_ctrl_c() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        let _ = ctrlc::set_handler(|| {});
    });
}

fn spawn_error(err: &std::io::Error) -> i32 {
    eprintln!("could not run {}: {err}", claude_path().display());
    if err.kind() == std::io::ErrorKind::NotFound {
        eprintln!("install Claude Code and make sure `claude` is on PATH");
    }
    127
}

/// Run claude attached to this terminal and return its exit code.
pub fn run(args: &[String], env: &Env) -> i32 {
    ignore_ctrl_c();
    match command(args, env).status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(err) => spawn_error(&err),
    }
}

/// Hand the terminal over to claude for good. On Unix this replaces the ccw process.
pub fn exec(args: &[String], env: &Env) -> i32 {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = command(args, env).exec();
        spawn_error(&err)
    }
    #[cfg(not(unix))]
    {
        run(args, env)
    }
}

/// Run claude without a terminal and collect stdout + stderr. Gives up after `timeout`.
pub fn capture(args: &[String], env: &Env, timeout: Duration) -> String {
    let mut cmd = command(args, env);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let Ok(mut child) = cmd.spawn() else {
        return String::new();
    };
    let reader = |pipe: Option<Box<dyn Read + Send>>| {
        thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut p) = pipe {
                let _ = p.read_to_end(&mut buf);
            }
            buf
        })
    };
    let out = reader(
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let err = reader(
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );

    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < timeout => thread::sleep(Duration::from_millis(30)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
        }
    }
    let mut text = String::from_utf8_lossy(&out.join().unwrap_or_default()).into_owned();
    text.push_str(&String::from_utf8_lossy(&err.join().unwrap_or_default()));
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn adds_yolo_flags() {
        assert_eq!(
            with_yolo_inner(&v(&["-p", "hi"]), false),
            v(&["--allow-dangerously-skip-permissions", YOLO, "-p", "hi"])
        );
    }

    #[test]
    fn strips_leading_separator() {
        assert_eq!(
            with_yolo_inner(&v(&["--", "-p"]), false),
            v(&["--allow-dangerously-skip-permissions", YOLO, "-p"])
        );
    }

    #[test]
    fn safe_flag_disables_yolo() {
        assert_eq!(with_yolo_inner(&v(&["--safe", "-p"]), false), v(&["-p"]));
        assert_eq!(with_yolo_inner(&v(&["-p"]), true), v(&["-p"]));
    }

    #[test]
    fn does_not_duplicate_yolo() {
        assert_eq!(with_yolo_inner(&v(&[YOLO]), false), v(&[YOLO]));
    }
}
