//! The pickers: account, then model, then effort.

use std::io::{self, BufRead, IsTerminal, Write};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal;

use crate::actions::{self, LoginOpts};
use crate::claude;
use crate::launch::{self, Choice, EFFORT_LEVELS};
use crate::profile::{self, account_name, describe_all, Profile};
use crate::style::{c, pad_end};
use crate::usage::{self, Loader, Slot};
use std::collections::HashMap;
use std::time::Duration;

enum Key {
    Up,
    Down,
    Enter,
    Quit,
    Esc,
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
    read_key_timeout(None).map(|key| key.unwrap_or(Key::Other))
}

/// Read one key. With a timeout, `None` means nothing arrived in time (used to redraw
/// while the usage fetches are still running).
fn read_key_timeout(timeout: Option<Duration>) -> io::Result<Option<Key>> {
    let _raw = RawMode::on()?;
    loop {
        if let Some(limit) = timeout {
            if !event::poll(limit)? {
                return Ok(None);
            }
        }
        let Event::Key(k) = event::read()? else {
            continue;
        };
        if k.kind != KeyEventKind::Press {
            continue;
        }
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        return Ok(Some(match k.code {
            KeyCode::Up => Key::Up,
            KeyCode::Down => Key::Down,
            KeyCode::Enter => Key::Enter,
            KeyCode::Esc => Key::Esc,
            KeyCode::Char('c') if ctrl => Key::Quit,
            KeyCode::Char('n') if ctrl => Key::Down,
            KeyCode::Char('p') if ctrl => Key::Up,
            KeyCode::Char(ch) if ch.is_ascii_digit() => Key::Num(ch as usize - '0' as usize),
            KeyCode::Char(ch) => Key::Char(ch.to_ascii_lowercase()),
            _ => Key::Other,
        }));
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

fn is_tui() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

// ---------------------------------------------------------------- plain fallback

/// One prompt over a numbered list. `default` is the 1-based row used for an empty answer.
fn pick_index_plain(question: &str, rows: &[String], default: usize) -> Option<usize> {
    for (i, row) in rows.iter().enumerate() {
        println!("{}  {row}", i + 1);
    }
    let answer = prompt_line(&format!("{question} 1-{} [{default}]: ", rows.len()))?;
    if answer.is_empty() {
        return Some(default);
    }
    match answer.parse::<usize>() {
        Ok(i) if i >= 1 && i <= rows.len() => Some(i),
        _ => {
            eprintln!("invalid choice");
            None
        }
    }
}

fn pick_plain(profiles: Vec<Profile>, loader: &mut Loader) -> Option<Picked> {
    loader.settle(Duration::from_secs(4));
    let slots = usage_snapshot(loader);
    let idx = pick_index_plain("account", &row_titles(&profiles, &slots), 1)?;
    let chosen = profiles.into_iter().nth(idx - 1)?;
    let remembered = launch::read(&chosen.name);
    let mut choice = remembered.clone();

    let model_rows = model_rows(&remembered);
    let model_default = model_cursor(&remembered, &model_rows) + 1;
    let labels: Vec<String> = model_rows
        .iter()
        .map(|(key, note)| format!("{key} — {note}"))
        .collect();
    let picked = pick_index_plain("model", &labels, model_default)?;
    let key = model_rows.get(picked - 1).map(|(key, _)| key.clone())?;
    choice.model = if key == "default" { None } else { Some(key) };

    let effort_rows = effort_rows(&remembered);
    let effort_default = effort_cursor(&remembered, &effort_rows) + 1;
    let labels: Vec<String> = effort_rows
        .iter()
        .map(|(key, note)| format!("{key} — {note}"))
        .collect();
    let picked = pick_index_plain("effort", &labels, effort_default)?;
    choice.effort = match effort_rows.get(picked - 1) {
        Some((key, _)) if key != "default" => Some(key.clone()),
        _ => None,
    };
    if let Err(e) = launch::write(&chosen.name, &choice) {
        eprintln!("could not remember the choice: {e}");
    }

    Some(Picked {
        profile: chosen,
        choice,
    })
}

fn row_titles(profiles: &[Profile], slots: &HashMap<String, Slot>) -> Vec<String> {
    profiles
        .iter()
        .map(|p| profile::picker_line(p, &usage_cell(slots.get(&p.name))))
        .collect()
}

// ---------------------------------------------------------------- list paging

enum Step {
    Chosen(usize),
    Custom,
    Back,
    Quit,
}

/// Draw a two-level list and read one answer. `footer` is the key legend; `custom_key`
/// enables a key that reports `Step::Custom`.
fn choose(
    title: &str,
    subtitle: Option<&str>,
    rows: &[(String, String)],
    cursor: &mut usize,
    footer: &str,
    custom_key: Option<char>,
) -> io::Result<Step> {
    let c = c();
    loop {
        clear_screen();
        let mut out = vec![title.to_string()];
        if let Some(sub) = subtitle {
            out.push(format!("{}{sub}{}", c.dim, c.reset));
        }
        out.push(String::new());
        for (i, (head, note)) in rows.iter().enumerate() {
            let sel = if i == *cursor {
                format!("{}❯{}", c.green, c.reset)
            } else {
                " ".to_string()
            };
            out.push(format!(
                " {sel} {}{}{}  {}{}{}  {}{}{}",
                c.dim,
                i + 1,
                c.reset,
                c.bold,
                pad_end(head, 12),
                c.reset,
                c.dim,
                note,
                c.reset
            ));
        }
        out.push(String::new());
        out.push(format!("{}{footer}{}", c.dim, c.reset));
        println!("{}", out.join("\n"));
        let _ = io::stdout().flush();

        let key = read_key()?;
        let len = rows.len().max(1);
        match key {
            Key::Quit | Key::Char('q') => {
                clear_screen();
                show_cursor();
                return Ok(Step::Quit);
            }
            Key::Esc => {
                clear_screen();
                show_cursor();
                return Ok(Step::Back);
            }
            Key::Up | Key::Char('k') => *cursor = (*cursor + len - 1) % len,
            Key::Down | Key::Char('j') => *cursor = (*cursor + 1) % len,
            Key::Num(n) if n >= 1 && n <= len => {
                clear_screen();
                show_cursor();
                return Ok(Step::Chosen(n - 1));
            }
            Key::Char(ch) if Some(ch) == custom_key => {
                clear_screen();
                return Ok(Step::Custom);
            }
            Key::Enter => {
                clear_screen();
                show_cursor();
                return Ok(Step::Chosen(*cursor));
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------- model step

/// Row for a remembered model, adding a row for a custom ID that is not in the list.
fn model_rows(choice: &Choice) -> Vec<(String, String)> {
    let mut rows: Vec<(String, String)> = launch::MODELS
        .iter()
        .map(|(key, note)| (key.to_string(), note.to_string()))
        .collect();
    if let Some(model) = &choice.model {
        if !launch::MODELS.iter().any(|(key, _)| key == model) {
            rows.push((model.clone(), "remembered custom model".to_string()));
        }
    }
    rows
}

fn model_cursor(choice: &Choice, rows: &[(String, String)]) -> usize {
    let key = choice.model.as_deref().unwrap_or("default");
    rows.iter().position(|(k, _)| k == key).unwrap_or(0)
}

fn custom_model_prompt(current: &Choice) -> Option<Option<String>> {
    show_cursor();
    clear_screen();
    let hint = current.model.as_deref().unwrap_or("default");
    let answer = prompt_line(&format!(
        "model alias or full ID [{hint}] (blank = default): "
    ))?;
    if answer.is_empty() {
        return Some(current.model.clone());
    }
    Some(launch::parse_model(&answer))
}

// ---------------------------------------------------------------- effort step

fn effort_rows(choice: &Choice) -> Vec<(String, String)> {
    let mut rows = vec![("default".to_string(), "let Claude Code decide".to_string())];
    rows.extend(
        EFFORT_LEVELS
            .iter()
            .map(|level| (level.to_string(), launch::effort_note(level).to_string())),
    );
    if let Some(effort) = &choice.effort {
        if !EFFORT_LEVELS.iter().any(|level| level == effort) {
            rows.push((effort.clone(), "remembered custom level".to_string()));
        }
    }
    rows
}

fn effort_cursor(choice: &Choice, rows: &[(String, String)]) -> usize {
    let key = choice.effort.as_deref().unwrap_or("default");
    rows.iter().position(|(k, _)| k == key).unwrap_or(0)
}

// ---------------------------------------------------------------- usage column

/// Text for the usage column of one row: the two windows, or why they are missing.
pub fn usage_cell(slot: Option<&Slot>) -> String {
    usage::cell_for(slot)
}

fn usage_snapshot(loader: &Loader) -> HashMap<String, Slot> {
    loader.snapshot()
}

// ---------------------------------------------------------------- account step

fn index_of(profiles: &[Profile], name: &str) -> Option<usize> {
    profiles.iter().position(|p| p.name == name)
}

/// Pick an account. `esc` here quits (there is nothing to go back to).
fn pick_account(profiles: &mut Vec<Profile>, loader: &mut Loader) -> Option<usize> {
    let c = c();
    let mut cursor = profiles
        .iter()
        .enumerate()
        .max_by_key(|(i, p)| (p.last_used, std::cmp::Reverse(*i)))
        .map(|(i, _)| i)
        .unwrap_or(0);
    let mut status = String::new();
    let mut slots = usage_snapshot(loader);

    loop {
        clear_screen();
        let mut out = Vec::new();
        out.push(format!(
            "{}ccw{} {}— choose a Claude Code account{}  {}(5h / 7d window){}  ",
            c.bold, c.reset, c.dim, c.reset, c.dim, c.reset
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
                profile::picker_line(p, &usage_cell(slots.get(&p.name)))
            ));
        }
        out.push(String::new());
        let mode = if claude::is_safe() {
            format!("{}skip-permissions OFF{}", c.yellow, c.reset)
        } else {
            format!("{}YOLO{}", c.green, c.reset)
        };
        let busy = if loader.pending() {
            format!("{}· checking usage…{}", c.dim, c.reset)
        } else {
            String::new()
        };
        out.push(format!(
            "{}↑↓/jk move · enter next · l login · a add · e label · r refresh · u usage · q quit{}  {mode} {busy}",
            c.dim, c.reset
        ));
        if !status.is_empty() {
            out.push(format!("{}{status}{}", c.yellow, c.reset));
        }
        println!("{}", out.join("\n"));
        let _ = io::stdout().flush();

        // Redraw every so often while a fetch is still running, so the column fills in.
        let key = match read_key_timeout(Some(Duration::from_millis(300))) {
            Ok(Some(k)) => k,
            Ok(None) => {
                slots = usage_snapshot(loader);
                continue;
            }
            Err(e) => {
                eprintln!("terminal error: {e}");
                return None;
            }
        };
        status.clear();
        let len = profiles.len();
        match key {
            Key::Quit | Key::Char('q') | Key::Esc => {
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
                    return Some(n - 1);
                }
                status = format!("no profile {n}");
            }
            Key::Enter => {
                clear_screen();
                show_cursor();
                return Some(cursor);
            }
            Key::Char('u') => {
                show_cursor();
                loader.reload(profiles.iter().map(|p| p.name.clone()).collect(), true);
                slots = usage_snapshot(loader);
                status = "refreshing usage".into();
            }
            Key::Char('r') => {
                show_cursor();
                *profiles = describe_all(false);
                loader.reload(profiles.iter().map(|p| p.name.clone()).collect(), true);
                slots = usage_snapshot(loader);
                cursor = cursor.min(profiles.len().saturating_sub(1));
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
                *profiles = describe_all(false);
                cursor = index_of(profiles, &name).unwrap_or(0);
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
                *profiles = describe_all(false);
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
                *profiles = describe_all(false);
                if let Some(i) = index_of(profiles, &name) {
                    cursor = i;
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------- entry points

/// An account plus the model/effort it launches with.
pub struct Picked {
    pub profile: Profile,
    pub choice: Choice,
}

fn edit_model(profile: &Profile, choice: &mut Choice) -> bool {
    let c = c();
    let title = format!(
        "{}ccw{} {}— pick a model for {}{}{}",
        c.bold, c.reset, c.dim, c.bold, profile.name, c.reset
    );
    let footer = "↑↓/jk move · enter next · c custom ID · esc back · q quit";
    loop {
        let rows = model_rows(choice);
        let mut cursor = model_cursor(choice, &rows);
        match choose(&title, None, &rows, &mut cursor, footer, Some('c')) {
            Err(e) => {
                eprintln!("terminal error: {e}");
                return false;
            }
            Ok(Step::Quit) | Ok(Step::Back) => return false,
            Ok(Step::Custom) => {
                if ask_custom_model(choice) {
                    return true;
                }
            }
            Ok(Step::Chosen(i)) => {
                let Some((key, _)) = rows.get(i) else {
                    return false;
                };
                choice.model = if key == "default" {
                    None
                } else {
                    Some(key.clone())
                };
                return true;
            }
        }
    }
}

fn edit_effort(profile: &Profile, choice: &mut Choice) -> bool {
    let c = c();
    let title = format!(
        "{}ccw{} {}— pick an effort level for {}{}{}",
        c.bold, c.reset, c.dim, c.bold, profile.name, c.reset
    );
    let rows = effort_rows(choice);
    let mut cursor = effort_cursor(choice, &rows);
    let footer = "↑↓/jk move · enter launch · esc back · q quit";
    match choose(&title, None, &rows, &mut cursor, footer, None) {
        Err(e) => {
            eprintln!("terminal error: {e}");
            false
        }
        Ok(Step::Chosen(i)) => {
            choice.effort = match rows.get(i) {
                Some((key, _)) if key != "default" => Some(key.clone()),
                _ => None,
            };
            true
        }
        Ok(Step::Back) | Ok(Step::Quit) => false,
        Ok(Step::Custom) => false,
    }
}

/// Show the account picker, then model and effort for that account.
pub fn pick(initial: Vec<Profile>) -> Option<Picked> {
    let mut loader = Loader::start(initial.iter().map(|p| p.name.clone()).collect());
    if !is_tui() {
        return pick_plain(initial, &mut loader);
    }
    let mut profiles = initial;
    loop {
        let idx = pick_account(&mut profiles, &mut loader)?;
        let profile = profiles.get(idx)?.clone();
        let mut choice = launch::read(&profile.name);

        if !edit_model(&profile, &mut choice) {
            continue;
        }
        // A custom model is typed in, so the picker has to be shown again for it to be
        // listed; only the effort step follows from here.
        if edit_effort(&profile, &mut choice) {
            if let Err(e) = launch::write(&profile.name, &choice) {
                eprintln!("could not remember the choice: {e}");
            }
            return Some(Picked { profile, choice });
        }
        continue;
    }
}

/// Ask for a custom model ID from the header row without opening the list.
pub fn ask_custom_model(choice: &mut Choice) -> bool {
    match custom_model_prompt(choice) {
        Some(model) => {
            choice.model = model;
            true
        }
        None => false,
    }
}
