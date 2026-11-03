//! ccw - Claude Code multi-account launcher.

mod actions;
mod claude;
mod launch;
mod profile;
mod style;
mod tui;
mod usage;

use std::path::PathBuf;
use std::process::ExitCode;

use actions::parse_login_opts;
use launch::{parse_effort, parse_model, Choice};
use profile::{describe, describe_all, human_age, profile_env, DEFAULT};
use style::{c, pad, pad_end};

const USAGE: &str = "\
ccw - Claude Code multi-account launcher

Usage:
  ccw                      TUI: pick an account, then a model and effort, then launch claude
  ccw <name> [args...]     launch claude with that profile (no TUI)
  ccw list                 profiles with account, plan, label and last use
  ccw account [name]       one profile in detail (alias: whoami)
  ccw label [name] [text]  set the picker display name (no text = clear, no name = list)
  ccw defaults [name]      remembered model/effort (alias: model)
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

Launch flags (anywhere on the command line):
  --model <alias|id>       model for this session (opus, sonnet, haiku, fable, default, or an
                           Anthropic model ID). Skips the model step.
  --effort <level>         low, medium, high, xhigh, max or default. Skips the effort step.
  -Q, --quiet              do not print the launch summary line

The picker shows each account's 5h/7d rate-limit usage, read from Claude Code's
/api/oauth/usage endpoint in the background. CCW_NO_USAGE=1 turns that off.

Model and effort are remembered per profile in ~/.ccw/.launch/<name>; the flags above write
that memory too, so the picker starts where you left off.

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

/// `ccw`-only flags, removed from argv before claude sees what is left.
#[derive(Default)]
struct LaunchFlags {
    model: Option<String>,
    model_given: bool,
    effort: Option<String>,
    effort_given: bool,
    quiet: bool,
}

impl LaunchFlags {
    /// Overlay the explicit flags on a remembered choice.
    fn apply(&self, choice: &mut Choice) {
        if self.model_given {
            choice.model = self.model.clone();
        }
        if self.effort_given {
            choice.effort = self.effort.clone();
        }
    }

    fn any_given(&self) -> bool {
        self.model_given || self.effort_given
    }
}

/// Split --model/--effort/--quiet out of argv. Bad values are an error, not a panic.
fn parse_launch_flags(argv: &[String]) -> Result<(LaunchFlags, Vec<String>), String> {
    let mut flags = LaunchFlags::default();
    let mut rest = Vec::with_capacity(argv.len());
    let mut i = 0;
    while i < argv.len() {
        let arg = &argv[i];
        if arg == "--" {
            // everything after the separator belongs to claude
            rest.extend(argv[i..].iter().cloned());
            break;
        }
        let (name, inline) = match arg.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n, Some(v.to_string())),
            _ => (arg.as_str(), None),
        };
        let value = |i: &mut usize| -> Result<String, String> {
            if let Some(v) = &inline {
                return Ok(v.clone());
            }
            *i += 1;
            argv.get(*i)
                .cloned()
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match name {
            "--model" => {
                let raw = value(&mut i)?;
                flags.model_given = true;
                flags.model = parse_model(&raw);
            }
            "--effort" => {
                let raw = value(&mut i)?;
                flags.effort_given = true;
                flags.effort = parse_effort(&raw)?;
            }
            "-Q" | "--quiet" => flags.quiet = true,
            _ => rest.push(arg.clone()),
        }
        i += 1;
    }
    Ok((flags, rest))
}

/// claude arguments for a launch: model/effort first, then the YOLO flags and the
/// caller's own arguments.
fn claude_args(choice: &Choice, passthrough: &[String]) -> Vec<String> {
    let mut args = choice.args();
    args.extend(claude::with_yolo(passthrough));
    args
}

fn launch(name: &str, flags: &LaunchFlags, passthrough: &[String]) -> i32 {
    let mut choice = launch::read(name);
    flags.apply(&mut choice);
    if flags.any_given() {
        if let Err(e) = launch::write(name, &choice) {
            eprintln!("could not remember the choice: {e}");
        }
    }
    if !flags.quiet {
        let c = c();
        println!(
            "{}launching claude as {name} ({}){}",
            c.dim,
            choice.summary(),
            c.reset
        );
    }
    claude::exec(&claude_args(&choice, passthrough), &profile_env(name))
}

fn cmd_list() -> i32 {
    let c = c();
    let mut rows = describe_all(false);
    profile::sort_for_picker(&mut rows);
    let line_at = rows.iter().position(profile::needs_login);
    let show_label = rows.iter().any(|p| p.label.is_some());
    let usage_off = usage::disabled();
    // Measure first so the header and every row share one set of widths.
    let usage_cells: Vec<String> = rows
        .iter()
        .map(|p| {
            if profile::needs_login(p) {
                // no token, so nothing to show and nothing to fetch
                "-".to_string()
            } else if usage_off {
                "usage off".to_string()
            } else {
                let slot = usage::lookup(&p.name, true);
                usage::cell_for(Some(&slot))
            }
        })
        .collect();
    let ages: Vec<String> = rows.iter().map(|p| human_age(p.last_used)).collect();
    let models: Vec<String> = rows
        .iter()
        .map(|p| launch::read(&p.name).model.unwrap_or_else(|| "-".into()))
        .collect();
    let efforts: Vec<String> = rows
        .iter()
        .map(|p| launch::read(&p.name).effort.unwrap_or_else(|| "-".into()))
        .collect();

    let width_of = |items: &[String], cap: usize| {
        items
            .iter()
            .map(|s| s.chars().count())
            .chain([cap])
            .max()
            .unwrap_or(cap)
            .max(4)
    };
    let w_profile = width_of(&rows.iter().map(|p| p.name.clone()).collect::<Vec<_>>(), 16);
    let w_label = if show_label {
        width_of(
            &rows
                .iter()
                .map(|p| p.label.clone().unwrap_or_default())
                .collect::<Vec<_>>(),
            20,
        )
    } else {
        0
    };
    let w_account = width_of(
        &rows
            .iter()
            .map(|p| {
                p.email
                    .clone()
                    .or_else(|| p.auth.clone())
                    .unwrap_or_else(|| "-".into())
            })
            .collect::<Vec<_>>(),
        32,
    );
    let w_plan = width_of(
        &rows
            .iter()
            .map(|p| p.plan.clone().unwrap_or_default())
            .collect::<Vec<_>>(),
        8,
    );
    let w_usage = width_of(&usage_cells, usage::CELL_WIDTH).max(usage::CELL_WIDTH);
    let w_age = width_of(&ages, 9);
    let w_model = width_of(&models, 10);
    let w_effort = width_of(&efforts, 8);

    let mut head = vec![pad("profile", w_profile)];
    if show_label {
        head.push(pad("label", w_label));
    }
    head.push(pad("account", w_account));
    head.push(pad("plan", w_plan));
    head.push(pad("5h / 7d", w_usage));
    head.push(pad("last used", w_age));
    head.push(pad("model", w_model));
    head.push(pad("effort", w_effort));
    println!("{}{}{} path", c.dim, head.join(" "), c.reset);

    for (i, p) in rows.iter().enumerate() {
        if line_at == Some(i) {
            // a rule under the accounts that can launch, measured with the same columns
            let rule: usize = w_profile
                + w_account
                + w_plan
                + w_usage
                + w_age
                + w_model
                + w_effort
                + 6
                + if show_label { w_label + 1 } else { 0 };
            let label = " needs a login ";
            let left = rule.saturating_sub(label.chars().count()) / 2;
            let right = rule
                .saturating_sub(label.chars().count())
                .saturating_sub(left);
            println!(
                "{}{}{}{}{}{}{}",
                c.dim,
                "─".repeat(left),
                c.yellow,
                label,
                c.dim,
                "─".repeat(right),
                c.reset
            );
        }
        let mut cols = vec![pad(&p.name, w_profile)];
        if show_label {
            cols.push(pad(p.label.as_deref().unwrap_or(""), w_label));
        }
        cols.push(pad(
            p.email.as_deref().or(p.auth.as_deref()).unwrap_or("-"),
            w_account,
        ));
        cols.push(pad(p.plan.as_deref().unwrap_or(""), w_plan));
        cols.push(pad(&usage_cells[i], w_usage));
        cols.push(pad(&ages[i], w_age));
        cols.push(pad(&models[i], w_model));
        cols.push(pad(&efforts[i], w_effort));
        if profile::needs_login(p) {
            println!(
                "{}{}{}{}{}",
                c.dim,
                cols.join(" "),
                c.reset,
                p.dir.display(),
                c.reset
            );
        } else {
            println!("{}{}{}{}", cols.join(" "), c.dim, p.dir.display(), c.reset);
        }
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
    let choice = launch::read(name);
    println!("profile   {}", info.name);
    println!("label     {}", info.label.as_deref().unwrap_or("(none)"));
    println!("email     {}", dash(info.email.as_deref()));
    println!(
        "account   {}",
        dash(info.email.as_deref().map(profile::account_name))
    );
    println!("plan      {}", dash(info.plan.as_deref()));
    println!("auth      {}", dash(info.auth.as_deref()));
    println!("model     {}", dash(choice.model.as_deref()));
    println!("effort    {}", dash(choice.effort.as_deref()));
    println!("dir       {}", info.dir.display());
    println!("last used {}", human_age(info.last_used));
    0
}

fn cmd_launch_defaults(args: &[String], flags: &LaunchFlags) -> i32 {
    let c = c();
    let Some(name) = args.first() else {
        for p in describe_all(true) {
            println!("{:<16} {}", p.name, launch::read(&p.name).summary());
        }
        return 0;
    };
    if !is_known(name) {
        eprintln!("unknown profile \"{name}\"");
        return 1;
    }
    let mut choice = launch::read(name);
    flags.apply(&mut choice);
    if let Err(e) = launch::write(name, &choice) {
        eprintln!("could not save: {e}");
        return 1;
    }
    println!(
        "{}{name} launches with {}{}",
        c.green,
        choice.summary(),
        c.reset
    );
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
    let (flags, argv) = match parse_launch_flags(&argv) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    dispatch(argv, flags)
}

/// Route the command line once ccw's own flags are out of the way.
fn dispatch(argv: Vec<String>, flags: LaunchFlags) -> i32 {
    let first = argv.first().cloned().unwrap_or_default();
    let rest = argv.get(1..).unwrap_or_default();
    match first.as_str() {
        "list" | "ls" => return cmd_list(),
        "label" => return cmd_label(rest),
        "account" | "whoami" => {
            return cmd_account(rest.first().map(String::as_str).unwrap_or(DEFAULT))
        }
        "defaults" | "model" => return cmd_launch_defaults(rest, &flags),
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
            return launch(&first, &flags, rest);
        }
        eprintln!(
            "unknown profile \"{first}\". known: {}",
            profile::list_names().join(", ")
        );
        eprintln!("create it with: ccw add <name>   (or run `ccw` for the picker)");
        return 1;
    }

    // TUI path
    let Some(picked) = tui::pick(describe_all(false)) else {
        return 130;
    };
    let tui::Picked { profile, choice } = picked;
    if !profile.exists {
        let code = actions::add(&profile.name, &Default::default());
        if code != 0 {
            return code;
        }
    }
    let passthrough = if first == "--" { rest } else { &argv[..] };
    let mut choice = choice;
    flags.apply(&mut choice);
    if flags.any_given() {
        if let Err(e) = launch::write(&profile.name, &choice) {
            eprintln!("could not remember the choice: {e}");
        }
    }
    let c = c();
    if !flags.quiet {
        println!(
            "{}launching claude as {} ({}){}",
            c.dim,
            profile.name,
            choice.summary(),
            c.reset
        );
    }
    claude::exec(
        &claude_args(&choice, passthrough),
        &profile_env(&profile.name),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn launch_flags_are_split_out() {
        let (flags, rest) =
            parse_launch_flags(&v(&["work", "--model", "opus", "-p", "hi"])).expect("parses");
        assert_eq!(rest, v(&["work", "-p", "hi"]));
        assert_eq!(flags.model.as_deref(), Some("opus"));
        assert!(!flags.effort_given);
        assert!(!flags.quiet);
    }

    #[test]
    fn inline_and_equals_forms() {
        let (flags, rest) =
            parse_launch_flags(&v(&["--model=sonnet", "--effort=XHIGH", "work"])).expect("parses");
        assert_eq!(rest, v(&["work"]));
        assert_eq!(flags.model.as_deref(), Some("sonnet"));
        assert_eq!(flags.effort.as_deref(), Some("xhigh"));
    }

    #[test]
    fn model_default_clears_the_value() {
        let (flags, _) = parse_launch_flags(&v(&["--model", "default"])).expect("parses");
        assert!(flags.model_given);
        assert_eq!(flags.model, None);
    }

    #[test]
    fn bad_effort_is_an_error() {
        assert!(parse_launch_flags(&v(&["--effort", "turbo"])).is_err());
        assert!(parse_launch_flags(&v(&["--effort"])).is_err());
    }

    #[test]
    fn separator_stops_flag_parsing() {
        let (flags, rest) =
            parse_launch_flags(&v(&["work", "--", "--model", "haiku"])).expect("parses");
        assert_eq!(flags.model, None);
        assert!(!flags.model_given);
        assert_eq!(rest, v(&["work", "--", "--model", "haiku"]));
    }

    #[test]
    fn quiet_flag() {
        let (flags, rest) = parse_launch_flags(&v(&["-Q", "work"])).expect("parses");
        assert!(flags.quiet);
        assert_eq!(rest, v(&["work"]));
    }

    #[test]
    fn flags_overlay_the_remembered_choice() {
        let mut choice = Choice {
            model: Some("sonnet".into()),
            effort: Some("low".into()),
        };
        let (flags, _) = parse_launch_flags(&v(&["--model", "opus"])).expect("parses");
        flags.apply(&mut choice);
        assert_eq!(choice.model.as_deref(), Some("opus"));
        assert_eq!(choice.effort.as_deref(), Some("low"));
    }

    #[test]
    fn claude_args_put_model_first() {
        let choice = Choice {
            model: Some("opus".into()),
            effort: Some("high".into()),
        };
        let args = claude_args(&choice, &v(&["-p", "hi"]));
        assert_eq!(args[0], "--model");
        assert_eq!(args[1], "opus");
        assert_eq!(args[2], "--effort");
        assert_eq!(args[3], "high");
        assert!(args.contains(&"-p".to_string()));
    }
}
