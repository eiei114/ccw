//! The account picker.

use std::io::{self, BufRead, IsTerminal, Write};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal;

use crate::actions::{self, LoginOpts};
use crate::claude;
use crate::profile::{self, account_name, describe_all, Profile};
use crate::style::{c, pad_end};

enum Key {
    Up,
    Down,
    Enter,
    Quit,
    Num(usize),
    Char(char),
    Other,
}

struct RawMode;

impl RawMode {
    fn on() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        Ok(RawMode)
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
    }
}

fn read_key() -> io::Result<Key> {
    let _raw = RawMode::on()?;
    loop {
        let Event::Key(k) = event::read()? else {
            continue;
        };
        if k.kind != KeyEventKind::Press {
            continue;
        }
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        return Ok(match k.code {
            KeyCode::Up => Key::Up,
            KeyCode::Down => Key::Down,
            KeyCode::Enter => Key::Enter,
            KeyCode::Esc => Key::Quit,
            KeyCode::Char('c') if ctrl => Key::Quit,
            KeyCode::Char('n') if ctrl => Key::Down,
            KeyCode::Char('p') if ctrl => Key::Up,
            KeyCode::Char(ch) if ch.is_ascii_digit() => Key::Num(ch as usize - '0' as usize),
            KeyCode::Char(ch) => Key::Char(ch.to_ascii_lowercase()),
            _ => Key::Other,
        });
    }
}

/// Print a question and read one line. None on EOF (or Ctrl-C/Ctrl-Z at the prompt).
pub fn prompt_line(question: &str) -> Option<String> {
    print!("{question}");
    let _ = io::stdout().flush();
    let mut line = String::new();
    match io::stdin().lock().read_line(&mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(line.trim().to_string()),
    }
}

fn clear_screen() {
    print!("\x1b[2J\x1b[H");
    let _ = io::stdout().flush();
}

fn show_cursor() {
    print!("{}", c().show_cursor);
    let _ = io::stdout().flush();
}

/// Fallback for non-TTY stdin (CI, piped input): a plain numbered list.
fn pick_plain(profiles: Vec<Profile>) -> Option<Profile> {
    println!("#  profile        account");
    for (i, p) in profiles.iter().enumerate() {
        let who = p.email.as_deref().or(p.auth.as_deref()).unwrap_or("-");
        let plan = p
            .plan
            .as_deref()
            .map(|pl| format!(" ({pl})"))
            .unwrap_or_default();
        println!("{}  {} {who}{plan}", i + 1, pad_end(&p.name, 14));
    }
    let answer = prompt_line(&format!("pick 1-{} [1]: ", profiles.len())).unwrap_or_default();
    let idx = if answer.is_empty() {
        Some(1)
    } else {
        answer.parse::<usize>().ok()
    };
    match idx {
        Some(i) if i >= 1 && i <= profiles.len() => profiles.into_iter().nth(i - 1),
        _ => {
            eprintln!("invalid choice");
            None
        }
    }
}

fn index_of(profiles: &[Profile], name: &str) -> Option<usize> {
    profiles.iter().position(|p| p.name == name)
}

/// Show the picker and return the chosen profile, or None when the user quits.
pub fn pick(initial: Vec<Profile>) -> Option<Profile> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return pick_plain(initial);
    }
    let c = c();
    let mut profiles = initial;
    let mut cursor = profiles
        .iter()
        .enumerate()
        .max_by_key(|(i, p)| (p.last_used, std::cmp::Reverse(*i)))
        .map(|(i, _)| i)
        .unwrap_or(0);
    let mut status = String::new();

    loop {
        clear_screen();
        let mut out = Vec::new();
        out.push(format!(
            "{}ccw{} {}— choose a Claude Code account{}",
            c.bold, c.reset, c.dim, c.reset
        ));
        out.push(String::new());
        for (i, p) in profiles.iter().enumerate() {
            let sel = if i == cursor {
                format!("{}❯{}", c.green, c.reset)
            } else {
                " ".to_string()
            };
            out.push(format!(
                " {sel} {}{}{}  {}",
                c.dim,
                i + 1,
                c.reset,
                profile::picker_line(p)
            ));
        }
        out.push(String::new());
        let mode = if claude::is_safe() {
            format!("{}skip-permissions OFF{}", c.yellow, c.reset)
        } else {
            format!("{}YOLO{}", c.green, c.reset)
        };
        out.push(format!(
            "{}↑↓/jk move · enter launch · l login · a add · e label · r refresh · q quit{}  {mode}",
            c.dim, c.reset
        ));
        if !status.is_empty() {
            out.push(format!("{}{status}{}", c.yellow, c.reset));
        }
        println!("{}", out.join("\n"));
        let _ = io::stdout().flush();

        let key = match read_key() {
            Ok(k) => k,
            Err(e) => {
                eprintln!("terminal error: {e}");
                return None;
            }
        };
        status.clear();
        let len = profiles.len();
        match key {
            Key::Quit | Key::Char('q') => {
                clear_screen();
                show_cursor();
                return None;
            }
            Key::Up | Key::Char('k') => cursor = (cursor + len - 1) % len,
            Key::Down | Key::Char('j') => cursor = (cursor + 1) % len,
            Key::Num(n) => {
                if n >= 1 && n <= len {
                    clear_screen();
                    show_cursor();
                    return profiles.into_iter().nth(n - 1);
                }
                status = format!("no profile {n}");
            }
            Key::Enter => {
                clear_screen();
                show_cursor();
                return profiles.into_iter().nth(cursor);
            }
            Key::Char('r') => {
                show_cursor();
                profiles = describe_all(false);
                cursor = cursor.min(profiles.len() - 1);
            }
            Key::Char('l') => {
                show_cursor();
                clear_screen();
                let target = &profiles[cursor];
                let name = target.name.clone();
                let current = target
                    .email
                    .as_deref()
                    .map(|e| format!(" [{e}]"))
                    .unwrap_or_default();
                let Some(answer) = prompt_line(&format!(
                    "email to pre-fill for {name}{current} (blank = any): "
                )) else {
                    status = "cancelled".into();
                    continue;
                };
                let email = if profile::is_email(&answer) {
                    Some(answer)
                } else {
                    target.email.clone()
                };
                let code = actions::login(
                    &name,
                    &LoginOpts {
                        email,
                        ..Default::default()
                    },
                );
                profiles = describe_all(false);
                cursor = index_of(&profiles, &name).unwrap_or(0);
                status = if code == 0 {
                    format!("logged in {name}")
                } else {
                    format!("login exited {code}")
                };
            }
            Key::Char('e') => {
                show_cursor();
                clear_screen();
                let target = &profiles[cursor];
                let name = target.name.clone();
                let hint = target
                    .email
                    .as_deref()
                    .map(|e| format!(" [{}]", account_name(e)))
                    .unwrap_or_default();
                let Some(label) = prompt_line(&format!("label for {name}{hint}: ")) else {
                    status = "cancelled".into();
                    continue;
                };
                status = match profile::write_label(&name, &label) {
                    Ok(()) if label.is_empty() => "label cleared".into(),
                    Ok(()) => format!("label set to {label}"),
                    Err(e) => format!("could not save label: {e}"),
                };
                profiles = describe_all(false);
            }
            Key::Char('a') => {
                show_cursor();
                clear_screen();
                let answer =
                    prompt_line("new profile: name, or email to sign in as <you@example.com>: ")
                        .unwrap_or_default();
                if answer.is_empty() {
                    status = "cancelled".into();
                    continue;
                }
                let (name, code) = if profile::is_email(&answer) {
                    let (name, code) = actions::add_for_email(&answer, &LoginOpts::default());
                    status = format!("added {name} for {answer}");
                    (name, code)
                } else {
                    let code = actions::add(&answer, &LoginOpts::default());
                    status = format!("profile {answer} added");
                    (answer, code)
                };
                if code != 0 {
                    status = format!("add {name} exited {code}");
                }
                profiles = describe_all(false);
                if let Some(i) = index_of(&profiles, &name) {
                    cursor = i;
                }
            }
            _ => {}
        }
    }
}
