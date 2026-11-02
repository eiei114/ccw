//! Commands that change profiles: add, login, rename, clone, trust.

use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};

use crate::claude;
use crate::profile::{self, describe, profile_dir, profile_env, DEFAULT};
use crate::style::c;

#[derive(Default, Clone)]
pub struct LoginOpts {
    pub email: Option<String>,
    pub console: bool,
    pub sso: bool,
}

/// Split --email/--console/--sso out of `args`. Returns the options and the leftover
/// positional arguments.
pub fn parse_login_opts(args: &[String]) -> (LoginOpts, Vec<String>) {
    let mut opts = LoginOpts::default();
    let mut rest = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--email" {
            opts.email = it.next().cloned();
        } else if let Some(v) = a.strip_prefix("--email=") {
            opts.email = Some(v.to_string());
        } else if a == "--console" || a == "--api" {
            opts.console = true;
        } else if a == "--sso" {
            opts.sso = true;
        } else if !a.starts_with('-') {
            rest.push(a.clone());
        }
    }
    if opts.email.as_deref().is_some_and(|e| !profile::is_email(e)) {
        opts.email = None;
    }
    (opts, rest)
}

fn login_args(opts: &LoginOpts) -> Vec<String> {
    let mut args = vec!["auth".to_string(), "login".to_string()];
    if opts.console {
        args.push("--console".into());
    }
    if opts.sso {
        args.push("--sso".into());
    }
    if let Some(e) = &opts.email {
        args.push("--email".into());
        args.push(e.clone());
    }
    args
}

fn warn_if_other_account(
    name: &str,
    wanted: Option<&str>,
    got: Option<&profile::Profile>,
    hint: bool,
) {
    let c = c();
    let (Some(wanted), Some(got)) = (wanted, got.and_then(|p| p.email.as_deref())) else {
        return;
    };
    if got.eq_ignore_ascii_case(wanted) {
        return;
    }
    println!(
        "{}warning: signed in as {got}, not {wanted}.{}",
        c.yellow, c.reset
    );
    if hint {
        println!(
            "{}if that is wrong, run: ccw login {name} --email {wanted}{}",
            c.dim, c.reset
        );
    }
}

fn run_login_for(name: &str, opts: &LoginOpts) -> i32 {
    let c = c();
    let dir = profile_dir(name);
    if let Err(e) = fs::create_dir_all(dir.join("projects")) {
        eprintln!("could not create {}: {e}", dir.display());
        return 1;
    }
    let as_email = opts
        .email
        .as_deref()
        .map(|e| format!(" as {e}"))
        .unwrap_or_default();
    println!("{}created {}{}", c.green, dir.display(), c.reset);
    println!("logging in{as_email}... (finish the browser flow, then you are back here)\n");
    let code = claude::run(&login_args(opts), &profile_env(name));
    let info = summarize(name);
    warn_if_other_account(name, opts.email.as_deref(), info.as_ref(), true);
    code
}

/// After a login, tell the user who they signed in as and offer a readable label.
fn summarize(name: &str) -> Option<profile::Profile> {
    let c = c();
    let info = describe(name, false);
    let Some(email) = info.email.clone() else {
        println!(
            "{}could not read the account for \"{name}\" yet{}",
            c.yellow, c.reset
        );
        return None;
    };
    let plan = info
        .plan
        .as_deref()
        .map(|p| format!(" ({p})"))
        .unwrap_or_default();
    println!(
        "\n{}profile \"{name}\" is now signed in as {email}{plan}{}",
        c.green, c.reset
    );
    let suggested = profile::account_name(&email);
    let current = info.label.as_deref().unwrap_or(name);
    if !suggested.is_empty() && current != suggested {
        println!(
            "{}shown as \"{current}\" in the picker. Run `ccw label {name} {suggested}` (or press e) to show \"{suggested}\" instead.{}",
            c.dim, c.reset
        );
    }
    if info.auth.as_deref() == Some("logged-out") {
        println!("{}warning: still not logged in{}", c.yellow, c.reset);
    }
    Some(info)
}

pub fn add(name: &str, opts: &LoginOpts) -> i32 {
    let c = c();
    if name == DEFAULT {
        eprintln!("\"default\" is reserved for ~/.claude");
        return 1;
    }
    if name.starts_with('.') || name.contains(['/', '\\']) {
        eprintln!("invalid profile name \"{name}\"");
        return 1;
    }
    let dir = profile_dir(name);
    if profile::dir_is_nonempty(&dir) {
        println!(
            "{}profile \"{name}\" already exists at {}{}",
            c.yellow,
            dir.display(),
            c.reset
        );
        return 1;
    }
    run_login_for(name, opts)
}

/// `ccw add` for an email: profile named after the address.
pub fn add_for_email(email: &str, opts: &LoginOpts) -> (String, i32) {
    let name = profile::unique_name(&profile::profile_name_from_email(email));
    let opts = LoginOpts {
        email: Some(email.to_string()),
        ..opts.clone()
    };
    let code = add(&name, &opts);
    (name, code)
}

fn base36(mut n: u128) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = Vec::new();
    loop {
        out.push(DIGITS[(n % 36) as usize]);
        n /= 36;
        if n == 0 {
            break;
        }
    }
    out.reverse();
    String::from_utf8(out).unwrap()
}

/// `ccw add` with no name: log in first, then name the profile after the account.
pub fn add_auto(opts: &LoginOpts) -> i32 {
    let c = c();
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let stamp = base36(millis);
    let temp = format!("new-{}", &stamp[stamp.len().saturating_sub(5)..]);
    let code = run_login_for(&temp, opts);
    if !profile_dir(&temp).exists() {
        return code;
    }
    let info = describe(&temp, false);
    let Some(email) = info.email else {
        println!(
            "{}logged in but the account is unknown; profile kept as \"{temp}\". Rename it with `ccw rename {temp} <name>`.{}",
            c.yellow, c.reset
        );
        return 0;
    };
    let target = profile::unique_name(&profile::profile_name_from_email(&email));
    if let Err(e) = fs::rename(profile_dir(&temp), profile_dir(&target)) {
        eprintln!("could not rename {temp} to {target}: {e}");
        return 1;
    }
    let _ = profile::write_label(&target, profile::account_name(&email));
    println!(
        "{}named the profile \"{target}\" (from {email}){}",
        c.green, c.reset
    );
    println!(
        "{}rename with: ccw rename {target} <name>{}",
        c.dim, c.reset
    );
    0
}

pub fn login(name: &str, opts: &LoginOpts) -> i32 {
    let c = c();
    if !profile::list_names().iter().any(|n| n == name) {
        eprintln!("unknown profile \"{name}\"");
        return 1;
    }
    if let Some(e) = &opts.email {
        println!("{}pre-filling {e} on the login page{}", c.dim, c.reset);
    }
    let code = claude::run(&login_args(opts), &profile_env(name));
    let info = summarize(name);
    warn_if_other_account(name, opts.email.as_deref(), info.as_ref(), false);
    code
}

/// `ccw login <email>`: reuse the profile that owns the address, or create one.
pub fn login_email(email: &str, opts: &LoginOpts) -> i32 {
    let c = c();
    let opts = LoginOpts {
        email: Some(email.to_string()),
        ..opts.clone()
    };
    let found = profile::describe_all(true).into_iter().find(|p| {
        p.email
            .as_deref()
            .is_some_and(|e| e.eq_ignore_ascii_case(email))
    });
    if let Some(p) = found {
        println!(
            "{}\"{email}\" already has profile \"{}\"{}",
            c.dim, p.name, c.reset
        );
        return login(&p.name, &opts);
    }
    let name = profile::unique_name(&profile::profile_name_from_email(email));
    println!(
        "{}no profile matches that email; creating \"{name}\"{}",
        c.dim, c.reset
    );
    add(&name, &opts)
}

pub fn rename(from: &str, to: &str) -> i32 {
    let c = c();
    if from == DEFAULT || to == DEFAULT {
        eprintln!("\"default\" is reserved for ~/.claude and cannot be renamed");
        return 1;
    }
    let (src, dst) = (profile_dir(from), profile_dir(to));
    if !src.exists() {
        eprintln!("profile \"{from}\" does not exist");
        return 1;
    }
    if dst.exists() {
        eprintln!("profile \"{to}\" already exists at {}", dst.display());
        return 1;
    }
    if let Err(e) = fs::rename(&src, &dst) {
        eprintln!("could not rename: {e}");
        return 1;
    }
    if let Some(label) = profile::read_label(from) {
        let _ = profile::write_label(to, &label);
        let _ = profile::write_label(from, "");
    }
    println!(
        "{}renamed {from} -> {to}{}  {}",
        c.green,
        c.reset,
        dst.display()
    );
    0
}

fn write_json(path: &Path, value: &Value) -> std::io::Result<()> {
    let text = serde_json::to_string_pretty(value).map_err(std::io::Error::other)?;
    fs::write(path, text)
}

/// Copy an existing profile's auth (and settings) into a new one.
pub fn clone(src: &str, dst: &str, no_auth: bool) -> i32 {
    if dst == DEFAULT {
        eprintln!("\"default\" is reserved for ~/.claude");
        return 1;
    }
    let src_dir = profile_dir(src);
    if !src_dir.exists() {
        eprintln!("source profile \"{src}\" does not exist");
        return 1;
    }
    let target = profile_dir(dst);
    if profile::dir_is_nonempty(&target) {
        eprintln!("target \"{dst}\" already exists at {}", target.display());
        return 1;
    }
    if let Err(e) = fs::create_dir_all(target.join("projects")) {
        eprintln!("could not create {}: {e}", target.display());
        return 1;
    }
    let mut copied: Vec<&str> = Vec::new();
    let mut files = vec!["settings.json", "settings.local.json"];
    if !no_auth {
        files.insert(0, ".credentials.json");
    }
    for file in files {
        let from = src_dir.join(file);
        if from.exists() && fs::copy(&from, target.join(file)).is_ok() {
            copied.push(file);
        }
    }

    // A minimal .claude.json so onboarding is skipped and workspace trust carries over.
    let src_json = [
        src_dir.join(".claude.json"),
        profile::paths().home.join(".claude.json"),
    ]
    .into_iter()
    .find(|p| p.exists());
    if let Some(o) = src_json
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
    {
        let mut projects = Map::new();
        if let Some(all) = o.get("projects").and_then(Value::as_object) {
            for (dir, cfg) in all {
                let Some(cfg) = cfg.as_object() else { continue };
                let entry: Map<String, Value> = [
                    "hasTrustDialogAccepted",
                    "allowedTools",
                    "hasCompletedProjectOnboarding",
                ]
                .iter()
                .filter_map(|k| cfg.get(*k).map(|v| (k.to_string(), v.clone())))
                .collect();
                if !entry.is_empty() {
                    projects.insert(dir.clone(), Value::Object(entry));
                }
            }
        }
        let mut minimal = Map::new();
        let mut keep = |k: &str| {
            if let Some(v) = o.get(k) {
                minimal.insert(k.to_string(), v.clone());
            }
        };
        keep("userID");
        if !no_auth {
            keep("oauthAccount");
        }
        minimal.insert("hasCompletedOnboarding".into(), Value::Bool(true));
        if let Some(v) = o.get("lastOnboardingVersion") {
            minimal.insert("lastOnboardingVersion".into(), v.clone());
        }
        minimal.insert("numStartups".into(), Value::from(1));
        if let Some(v) = o.get("installMethod") {
            minimal.insert("installMethod".into(), v.clone());
        }
        minimal.insert("projects".into(), Value::Object(projects));
        if write_json(&target.join(".claude.json"), &Value::Object(minimal)).is_ok() {
            copied.push(".claude.json");
        }
    }

    let c = c();
    let list = if copied.is_empty() {
        "empty".to_string()
    } else {
        copied.join(", ")
    };
    println!("{}cloned {src} -> {dst}{} ({list})", c.green, c.reset);
    if no_auth {
        println!("run `ccw login {dst}` to sign in, or `ccw {dst}` and use /login");
    } else {
        println!(
            "same account as {src}. Use `ccw login {dst}` (or /login inside) to switch account."
        );
    }
    0
}

/// Claude Code keys projects by the absolute path with forward slashes.
pub fn trust_key(dir: &Path) -> String {
    let s = dir.to_string_lossy().replace('\\', "/");
    s.strip_suffix('/').unwrap_or(&s).to_string()
}

/// Mark a directory trusted in a profile's .claude.json (silences "this workspace has not
/// been trusted" and lets settings.json permissions apply).
pub fn trust(name: &str, dir: &Path) -> i32 {
    let c = c();
    let cfg_path = profile::claude_json(name);
    let mut o = match fs::read_to_string(&cfg_path) {
        Ok(text) => match serde_json::from_str::<Value>(&text) {
            Ok(Value::Object(m)) => m,
            _ => {
                eprintln!(
                    "{} is not a JSON object; not touching it",
                    cfg_path.display()
                );
                return 1;
            }
        },
        Err(_) => Map::new(),
    };
    let key = trust_key(dir);
    let projects = o
        .entry("projects")
        .or_insert_with(|| Value::Object(Map::new()));
    if !projects.is_object() {
        *projects = Value::Object(Map::new());
    }
    let entry = projects
        .as_object_mut()
        .unwrap()
        .entry(key.clone())
        .or_insert_with(|| Value::Object(Map::new()));
    if !entry.is_object() {
        *entry = Value::Object(Map::new());
    }
    entry
        .as_object_mut()
        .unwrap()
        .insert("hasTrustDialogAccepted".into(), Value::Bool(true));
    if let Some(parent) = cfg_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Err(e) = write_json(&cfg_path, &Value::Object(o)) {
        eprintln!("could not write {}: {e}", cfg_path.display());
        return 1;
    }
    println!("{}trusted {key} in {name}{}", c.green, c.reset);
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn login_opts_split_from_positionals() {
        let (o, rest) = parse_login_opts(&v(&["--email", "me@corp.com", "work", "--sso"]));
        assert_eq!(o.email.as_deref(), Some("me@corp.com"));
        assert!(o.sso && !o.console);
        assert_eq!(rest, v(&["work"]));

        let (o, rest) = parse_login_opts(&v(&["work", "--email=bad", "--api"]));
        assert_eq!(o.email, None);
        assert!(o.console);
        assert_eq!(rest, v(&["work"]));
    }

    #[test]
    fn login_args_shape() {
        let opts = LoginOpts {
            email: Some("a@b.co".into()),
            console: true,
            sso: false,
        };
        assert_eq!(
            login_args(&opts),
            v(&["auth", "login", "--console", "--email", "a@b.co"])
        );
    }

    #[test]
    fn trust_keys_use_forward_slashes() {
        assert_eq!(trust_key(Path::new(r"C:\work\repo\")), "C:/work/repo");
        assert_eq!(trust_key(Path::new("/home/me/repo")), "/home/me/repo");
    }

    #[test]
    fn base36_matches_js() {
        assert_eq!(base36(0), "0");
        assert_eq!(base36(35), "z");
        assert_eq!(base36(36), "10");
    }
}
