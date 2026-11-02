//! ccw - Claude Code multi-account launcher.

mod actions;
mod claude;
mod profile;
mod style;
mod tui;

use std::path::PathBuf;
use std::process::ExitCode;

use actions::parse_login_opts;
use profile::{describe, describe_all, human_age, profile_env, DEFAULT};
use style::{c, pad, pad_end};

const USAGE: &str = "\
ccw - Claude Code multi-account launcher

Usage:
  ccw                      TUI: pick an account, then launch claude
  ccw <name> [args...]     launch claude with that profile (no TUI)
  ccw list                 profiles with account, plan, label and last use
  ccw account [name]       one profile in detail (alias: whoami)
  ccw label [name] [text]  set the picker display name (no text = clear, no name = list)
  ccw add                  log in first, then name the profile after the account
  ccw add <email>          sign in as that email (pre-filled), profile named after it
  ccw add <name> [--email <email>]
                           create a profile with a fixed name and log in
  ccw login <name> [--email <email>]
                           re-login an existing profile
  ccw login <email>        re-login the profile that owns that email (or create one)
  ccw clone <src> <dst> [--no-auth]
                           copy a profile (auth + settings) into a new one
  ccw rename <old> <new>   rename a profile directory
  ccw trust [name] [dir]   mark a directory trusted for that profile
  ccw -- <args...>         TUI, then pass args to claude (e.g. ccw -- -p \"hi\")
  ccw --version            print the version

Login flags: --email <addr>, --console (API billing), --sso

Every launch adds --dangerously-skip-permissions (YOLO mode).
Disable per call with --safe, or globally with CCW_SAFE=1.

Profiles live in ~/.ccw/<name> (override with CCW_HOME) and are isolated by
CLAUDE_CONFIG_DIR. The pseudo profile \"default\" uses the normal ~/.claude.
";

fn main() -> ExitCode {
    style::init_terminal();
    let code = run();
    ExitCode::from(code.clamp(0, 255) as u8)
}

fn is_known(name: &str) -> bool {
    profile::list_names().iter().any(|n| n == name)
}

fn cmd_list() -> i32 {
    let c = c();
    let rows = describe_all(false);
    let show_label = rows.iter().any(|p| p.label.is_some());
    let mut head = vec![("profile", 16)];
    if show_label {
        head.push(("label", 18));
    }
    head.extend([("account", 34), ("plan", 8)]);
    let line: Vec<String> = head.iter().map(|(h, w)| pad(h, *w)).collect();
    println!("{} last used path", line.join(" "));
    for p in &rows {
        let mut cols = vec![p.name.clone()];
        if show_label {
            cols.push(p.label.clone().unwrap_or_default());
        }
        cols.push(
            p.email
                .clone()
                .or_else(|| p.auth.clone())
                .unwrap_or_else(|| "-".into()),
        );
        cols.push(p.plan.clone().unwrap_or_default());
        let line: Vec<String> = cols
            .iter()
            .zip(&head)
            .map(|(col, (_, w))| pad(col, *w))
            .collect();
        println!(
            "{} {} {}{}{}",
            line.join(" "),
            pad_end(&human_age(p.last_used), 9),
            c.dim,
            p.dir.display(),
            c.reset
        );
    }
    0
}

fn cmd_label(args: &[String]) -> i32 {
    let c = c();
    let Some(name) = args.first() else {
        for p in describe_all(false) {
            let who = p.email.as_deref().or(p.auth.as_deref()).unwrap_or("-");
            println!(
                "{} {} {who}",
                pad_end(&p.name, 16),
                pad_end(p.label.as_deref().unwrap_or(""), 18)
            );
        }
        return 0;
    };
    if !is_known(name) {
        eprintln!("unknown profile \"{name}\"");
        return 1;
    }
    let label = args[1..].join(" ").trim().to_string();
    if let Err(e) = profile::write_label(name, &label) {
        eprintln!("could not save label: {e}");
        return 1;
    }
    if label.is_empty() {
        println!("{}label cleared for {name}{}", c.green, c.reset);
    } else {
        println!(
            "{}{name} now shows as \"{label}\" in the picker{}",
            c.green, c.reset
        );
    }
    0
}

fn cmd_account(name: &str) -> i32 {
    let info = describe(name, false);
    let dash = |v: Option<&str>| v.unwrap_or("-").to_string();
    println!("profile   {}", info.name);
    println!("label     {}", info.label.as_deref().unwrap_or("(none)"));
    println!("email     {}", dash(info.email.as_deref()));
    println!(
        "account   {}",
        dash(info.email.as_deref().map(profile::account_name))
    );
    println!("plan      {}", dash(info.plan.as_deref()));
    println!("auth      {}", dash(info.auth.as_deref()));
    println!("dir       {}", info.dir.display());
    println!("last used {}", human_age(info.last_used));
    0
}

fn run() -> i32 {
    let mut argv: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    // global flags usable anywhere on the command line
    if argv.iter().any(|a| a == "--safe" || a == "--no-yolo") {
        claude::set_safe();
        argv.retain(|a| a != "--safe" && a != "--no-yolo");
    }
    let first = argv.first().cloned().unwrap_or_default();
    let rest = argv.get(1..).unwrap_or_default();

    match first.as_str() {
        "list" | "ls" => return cmd_list(),
        "label" => return cmd_label(rest),
        "account" | "whoami" => {
            return cmd_account(rest.first().map(String::as_str).unwrap_or(DEFAULT))
        }
        "add" => {
            let (opts, pos) = parse_login_opts(rest);
            return match pos.first() {
                Some(t) if profile::is_email(t) => actions::add_for_email(t, &opts).1,
                Some(t) => actions::add(t, &opts),
                None => actions::add_auto(&opts),
            };
        }
        "login" => {
            let (opts, pos) = parse_login_opts(rest);
            return match (pos.first(), &opts.email) {
                (Some(t), _) if profile::is_email(t) => actions::login_email(t, &opts),
                (Some(t), _) => actions::login(t, &opts),
                (None, Some(e)) => actions::login_email(&e.clone(), &opts),
                (None, None) => {
                    eprintln!("usage: ccw login <name|email> [--email <email>]");
                    1
                }
            };
        }
        "rename" | "mv" => {
            return match rest {
                [from, to, ..] => actions::rename(from, to),
                _ => {
                    eprintln!("usage: ccw rename <old> <new>");
                    1
                }
            }
        }
        "clone" | "cp" => {
            let no_auth = rest.iter().any(|a| a == "--no-auth");
            let pos: Vec<&String> = rest.iter().filter(|a| !a.starts_with('-')).collect();
            return match pos.as_slice() {
                [src, dst, ..] => actions::clone(src, dst, no_auth),
                _ => {
                    eprintln!("usage: ccw clone <src> <dst> [--no-auth]");
                    1
                }
            };
        }
        "trust" => {
            let name = rest.first().map(String::as_str).unwrap_or(DEFAULT);
            let dir = match rest.get(1) {
                Some(d) => std::path::absolute(d).unwrap_or_else(|_| PathBuf::from(d)),
                None => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            };
            return actions::trust(name, &dir);
        }
        "help" | "-h" | "--help" => {
            print!("{USAGE}");
            return 0;
        }
        "-V" | "--version" | "version" => {
            println!("ccw {}", env!("CARGO_PKG_VERSION"));
            return 0;
        }
        _ => {}
    }

    // explicit profile: ccw <name> [claude args...]
    if !first.is_empty() && !first.starts_with('-') {
        if is_known(&first) {
            return claude::exec(&claude::with_yolo(rest), &profile_env(&first));
        }
        eprintln!(
            "unknown profile \"{first}\". known: {}",
            profile::list_names().join(", ")
        );
        eprintln!("create it with: ccw add <name>   (or run `ccw` for the picker)");
        return 1;
    }

    // TUI path
    let Some(chosen) = tui::pick(describe_all(false)) else {
        return 130;
    };
    if !chosen.exists {
        let code = actions::add(&chosen.name, &Default::default());
        if code != 0 {
            return code;
        }
    }
    let passthrough = if first == "--" { rest } else { &argv[..] };
    let c = c();
    println!("{}launching claude as {}{}", c.dim, chosen.name, c.reset);
    claude::exec(&claude::with_yolo(passthrough), &profile_env(&chosen.name))
}
