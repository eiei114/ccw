//! Profiles on disk: where they live, what account they hold, labels.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

use serde_json::Value;

use crate::claude::{self, Env};
use crate::style::{c, pad};

pub const DEFAULT: &str = "default";

pub struct Paths {
    pub home: PathBuf,
    /// Root for every non-default profile: `~/.ccw` (override with CCW_HOME).
    pub root: PathBuf,
    pub labels: PathBuf,
    /// Remembered model/effort per profile.
    pub prefs: PathBuf,
    /// Last known rate-limit usage per profile.
    pub usage: PathBuf,
    /// The normal Claude Code config dir, used by the "default" profile.
    pub default_dir: PathBuf,
}

pub fn paths() -> &'static Paths {
    static PATHS: OnceLock<Paths> = OnceLock::new();
    PATHS.get_or_init(|| {
        #[allow(deprecated)]
        let home = std::env::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let root = match std::env::var_os("CCW_HOME") {
            Some(v) if !v.is_empty() => PathBuf::from(v),
            _ => home.join(".ccw"),
        };
        Paths {
            labels: root.join(".labels"),
            prefs: root.join(".launch"),
            usage: root.join(".usage"),
            default_dir: home.join(".claude"),
            root,
            home,
        }
    })
}

pub fn profile_dir(name: &str) -> PathBuf {
    if name == DEFAULT {
        paths().default_dir.clone()
    } else {
        paths().root.join(name)
    }
}

/// `.claude.json` for a profile. The default profile keeps it in the home directory.
pub fn claude_json(name: &str) -> PathBuf {
    if name == DEFAULT {
        paths().home.join(".claude.json")
    } else {
        profile_dir(name).join(".claude.json")
    }
}

pub fn profile_env(name: &str) -> Env {
    if name == DEFAULT {
        Vec::new()
    } else {
        vec![("CLAUDE_CONFIG_DIR".to_string(), profile_dir(name))]
    }
}

pub fn list_names() -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(&paths().root)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| !n.starts_with('.'))
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names.insert(0, DEFAULT.to_string());
    names
}

pub fn dir_is_nonempty(dir: &Path) -> bool {
    fs::read_dir(dir)
        .map(|mut rd| rd.next().is_some())
        .unwrap_or(false)
}

// ------------------------------------------------------------------ labels

pub fn read_label(name: &str) -> Option<String> {
    let text = fs::read_to_string(paths().labels.join(name)).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// Set the picker display name. An empty label removes it.
pub fn write_label(name: &str, label: &str) -> std::io::Result<()> {
    let file = paths().labels.join(name);
    if label.is_empty() {
        let _ = fs::remove_file(file);
        return Ok(());
    }
    fs::create_dir_all(&paths().labels)?;
    fs::write(file, format!("{label}\n"))
}

// ------------------------------------------------------------ name helpers

pub fn is_email(s: &str) -> bool {
    let Some((local, domain)) = s.split_once('@') else {
        return false;
    };
    let bad = |p: &str| p.is_empty() || p.contains('@') || p.chars().any(char::is_whitespace);
    if bad(local) || bad(domain) {
        return false;
    }
    let bytes = domain.as_bytes();
    bytes
        .iter()
        .enumerate()
        .any(|(i, &b)| b == b'.' && i > 0 && i < bytes.len() - 1)
}

/// Local part of an email address.
pub fn account_name(email: &str) -> &str {
    match email.find('@') {
        Some(at) if at > 0 => &email[..at],
        _ => email,
    }
}

/// Lowercase, drop the email domain, collapse anything outside [a-z0-9._-] to '-'.
pub fn slugify(value: &str) -> String {
    let lower = value.to_lowercase();
    let base = lower.split('@').next().unwrap_or("");
    let mut out = String::new();
    let mut in_bad_run = false;
    for ch in base.chars() {
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '.' | '_' | '-') {
            out.push(ch);
            in_bad_run = false;
        } else if !in_bad_run {
            out.push('-');
            in_bad_run = true;
        }
    }
    out.trim_matches('-').chars().take(24).collect()
}

pub fn profile_name_from_email(email: &str) -> String {
    let slug = slugify(account_name(email));
    if slug.is_empty() {
        "account".to_string()
    } else {
        slug
    }
}

/// `base`, or `base-2`, `base-3`, ... when that directory is taken.
pub fn unique_name(base: &str) -> String {
    let mut target = base.to_string();
    let mut n = 2;
    while profile_dir(&target).exists() {
        target = format!("{base}-{n}");
        n += 1;
    }
    target
}

// ----------------------------------------------------------------- details

/// Everything the pickers show about one profile.
#[derive(Clone)]
pub struct Profile {
    pub name: String,
    pub dir: PathBuf,
    pub exists: bool,
    pub email: Option<String>,
    pub plan: Option<String>,
    pub auth: Option<String>,
    pub label: Option<String>,
    pub last_used: Option<SystemTime>,
}

fn last_used(name: &str) -> Option<SystemTime> {
    let dir = profile_dir(name);
    [claude_json(name), dir.join(".claude.json"), dir]
        .iter()
        .filter_map(|p| fs::metadata(p).and_then(|m| m.modified()).ok())
        .max()
}

/// Plan name from the cached oauthAccount block.
fn plan_from_account(acct: &Value) -> Option<String> {
    let s = |k: &str| acct.get(k).and_then(Value::as_str).unwrap_or("");
    let org = s("organizationType");
    let org = if org.len() >= 7 && org[..7].eq_ignore_ascii_case("claude_") {
        &org[7..]
    } else {
        org
    };
    if !org.is_empty() {
        return Some(org.to_string());
    }
    let tiers = format!("{} {}", s("organizationRateLimitTier"), s("seatTier")).to_lowercase();
    ["max", "pro", "team", "enterprise", "free"]
        .iter()
        .filter_map(|w| tiers.find(w).map(|i| (i, *w)))
        .min()
        .map(|(_, w)| w.to_string())
}

/// Read auth details from the profile's own cached files first, so the picker does not
/// spawn `claude` per profile. Falls back to `claude auth status` when the cache cannot
/// answer (token-only auth, first run after /login) unless `skip_cli` is set.
pub fn describe(name: &str, skip_cli: bool) -> Profile {
    let dir = profile_dir(name);
    let mut info = Profile {
        name: name.to_string(),
        exists: dir.exists(),
        dir,
        email: None,
        plan: None,
        auth: None,
        label: read_label(name),
        last_used: last_used(name),
    };
    if !info.exists {
        info.auth = Some("missing".into());
        return info;
    }

    let parsed: Option<Value> = fs::read_to_string(claude_json(name))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok());
    if let Some(acct) = parsed.as_ref().and_then(|v| v.get("oauthAccount")) {
        if let Some(email) = acct.get("emailAddress").and_then(Value::as_str) {
            if !email.is_empty() {
                info.email = Some(email.to_string());
                info.plan = plan_from_account(acct);
            }
        }
    }

    let has_creds = info.dir.join(".credentials.json").exists();
    info.auth = Some(
        if info.email.is_some() {
            "claude.ai"
        } else if has_creds {
            "unknown"
        } else {
            "logged-out"
        }
        .into(),
    );

    if !skip_cli && info.email.is_none() {
        let out = claude::capture(
            &["auth".into(), "status".into()],
            &profile_env(name),
            Duration::from_secs(20),
        );
        if let Some(json) = out
            .find('{')
            .and_then(|i| serde_json::from_str::<Value>(&out[i..]).ok())
        {
            let str_of = |k: &str| json.get(k).and_then(Value::as_str).map(String::from);
            info.auth = if json.get("loggedIn").and_then(Value::as_bool) == Some(true) {
                str_of("authMethod")
            } else {
                Some("logged-out".into())
            };
            if let Some(e) = str_of("email") {
                info.email = Some(e);
            }
            if let Some(p) = str_of("subscriptionType") {
                info.plan = Some(p);
            }
        }
    }
    info
}

pub fn describe_all(skip_cli: bool) -> Vec<Profile> {
    list_names().iter().map(|n| describe(n, skip_cli)).collect()
}

/// True when the profile cannot launch without signing in first: no credentials, or a
/// directory that is gone.
pub fn needs_login(p: &Profile) -> bool {
    !p.exists || matches!(p.auth.as_deref(), Some("logged-out") | Some("missing"))
}

/// Sink the accounts that need a login to the bottom, in place. The sort is stable, so the
/// picker keeps a familiar order above and below the line.
pub fn sort_for_picker(profiles: &mut [Profile]) {
    profiles.sort_by_key(needs_login);
}

pub fn human_age(t: Option<SystemTime>) -> String {
    let Some(t) = t else {
        return "never".into();
    };
    let secs = SystemTime::now()
        .duration_since(t)
        .unwrap_or_default()
        .as_secs_f64();
    let min = (secs / 60.0).round() as u64;
    if min < 1 {
        return "just now".into();
    }
    if min < 60 {
        return format!("{min}m ago");
    }
    let h = (min as f64 / 60.0).round() as u64;
    if h < 24 {
        return format!("{h}h ago");
    }
    format!("{}d ago", (h as f64 / 24.0).round() as u64)
}

/// Minimum width the email column needs before it is worth showing at all.
const MIN_EMAIL: usize = 16;

/// Column widths for one draw, chosen to fit the terminal.
#[derive(Clone, Copy)]
pub struct Columns {
    pub label: usize,
    pub email: usize,
    pub plan: usize,
    pub age: usize,
    /// Drop the reset time from the usage cell when the terminal is narrow.
    pub compact_usage: bool,
}

impl Default for Columns {
    fn default() -> Self {
        Columns {
            label: 16,
            email: 26,
            plan: 5,
            age: 8,
            compact_usage: false,
        }
    }
}

impl Columns {
    pub fn cell_width(&self) -> usize {
        crate::usage::cell_width(self.compact_usage)
    }

    /// Give up one column at a time, cheapest first: trim the label (the profile name is
    /// still on the right), drop the plan, drop the age, drop the email, shorten the label
    /// further, then shorten the usage cell. The email is all-or-nothing: a truncated
    /// address is not worth the column, so callers drop it before calling this.
    pub fn tighten(&mut self) -> bool {
        if self.label > 12 {
            self.label -= 2;
            true
        } else if self.plan > 0 {
            self.plan = 0;
            true
        } else if self.age > 0 {
            self.age = 0;
            true
        } else if self.email > 0 {
            self.email = 0;
            true
        } else if self.label > 6 {
            self.label -= 2;
            true
        } else if !self.compact_usage {
            self.compact_usage = true;
            true
        } else {
            false
        }
    }
}

/// Strip ANSI escapes so a rendered row can be measured.
fn visible_len(s: &str) -> usize {
    let mut count = 0;
    let mut in_escape = false;
    for ch in s.chars() {
        if in_escape {
            if ch.is_ascii_alphabetic() {
                in_escape = false;
            }
            continue;
        }
        if ch == '\x1b' {
            in_escape = true;
            continue;
        }
        count += 1;
    }
    count
}

/// One account's usage, rendered both ways so a narrow terminal can pick.
#[derive(Clone)]
pub struct UsageText {
    pub full: String,
    pub compact: String,
}

impl UsageText {
    pub fn from_slot(slot: Option<&crate::usage::Slot>) -> UsageText {
        use crate::usage::{cell, cell_compact, cell_for, Slot};
        let slot: Option<Slot> = slot.copied();
        let numbers = slot.and_then(|s| s.usage);
        match numbers {
            Some(usage) => UsageText {
                full: cell(&usage),
                compact: cell_compact(&usage),
            },
            None => {
                let reason = cell_for(slot.as_ref());
                UsageText {
                    full: reason.clone(),
                    compact: reason,
                }
            }
        }
    }
}

/// Render every picker row so that none is wider than `width` columns. Columns are given
/// up from the least important side until the widest row fits, so the terminal never wraps
/// a row onto the next line. The email column is sized to the longest address and dropped
/// entirely rather than shown as a fragment.
pub fn picker_rows(rows: &[(&Profile, UsageText)], width: usize) -> Vec<String> {
    // The email column needs to hold the longest address in the list, not an average.
    let widest_email = rows
        .iter()
        .filter_map(|(p, _)| p.email.as_deref().or(p.auth.as_deref()))
        .map(|e| e.chars().count())
        .max();
    let mut columns = Columns::default();
    if let Some(needed) = widest_email {
        if needed >= MIN_EMAIL {
            columns.email = needed;
        }
    }
    loop {
        let lines: Vec<String> = rows
            .iter()
            .map(|(p, usage)| picker_line(p, &usage.full, &usage.compact, &columns))
            .collect();
        let widest = lines.iter().map(|l| visible_len(l)).max().unwrap_or(0);
        // 3 columns for the lead gutter and the cursor
        if widest + 3 <= width || !columns.tighten() {
            return lines;
        }
    }
}
/// One row of the picker: identity on the left, rate-limit usage on the right, then the
/// plan and the last-used age. `usage` is the full cell and `usage_compact` the variant
/// without the reset time. Callers use [`picker_rows`] so the row fits the terminal.
pub fn picker_line(p: &Profile, usage: &str, usage_compact: &str, columns: &Columns) -> String {
    let c = c();
    let shown = p
        .label
        .clone()
        .or_else(|| p.email.as_deref().map(|e| account_name(e).to_string()))
        .unwrap_or_else(|| p.name.clone());
    let mut out = format!("{} ", pad(&shown, columns.label));
    if columns.email > 0 {
        let email = p.email.as_deref().or(p.auth.as_deref()).unwrap_or("");
        out.push_str(&format!(
            "{}{}{} ",
            c.cyan,
            pad(email, columns.email),
            c.reset
        ));
    }
    let text = if columns.compact_usage {
        usage_compact
    } else {
        usage
    };
    let cell = if needs_login(p) {
        let note = if p.exists { "logged out" } else { "not set up" };
        format!("{}{}{}", c.yellow, pad(note, columns.cell_width()), c.reset)
    } else {
        pad(text, columns.cell_width())
    };
    out.push_str(&cell);
    out.push(' ');
    if columns.plan > 0 {
        out.push_str(&format!(
            "{}{}{}",
            c.dim,
            pad(p.plan.as_deref().unwrap_or(""), columns.plan),
            c.reset
        ));
    }
    if columns.age > 0 {
        out.push_str(&format!(
            "{}{}{} ",
            c.dim,
            pad(&human_age(p.last_used), columns.age),
            c.reset
        ));
    }
    out.push_str(&format!("{}{}{}", c.dim, p.name, c.reset));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rows_fit_the_terminal() {
        let mut profile = Profile {
            name: "account-2".into(),
            dir: PathBuf::from("/tmp/account-2"),
            exists: true,
            email: Some("account@dotfield.xyz".into()),
            plan: Some("max".into()),
            auth: Some("claude.ai".into()),
            label: Some("account".into()),
            last_used: Some(SystemTime::now()),
        };
        let text = UsageText {
            full: "5h  16% · 7d  69%  ↻4h58m".to_string(),
            compact: "5h  16% · 7d  69%".to_string(),
        };
        for width in [40, 60, 80, 100, 132] {
            let rows = picker_rows(&[(&profile, text.clone())], width);
            let widest = rows.iter().map(|r| visible_len(r)).max().unwrap_or(0);
            assert!(widest + 3 <= width, "width {width} gave {widest}");
        }
        // narrow terminals keep the usage cell and drop the email
        let narrow = picker_rows(&[(&profile, text.clone())], 44);
        assert!(visible_len(&narrow[0]) + 3 <= 44);
        assert!(!narrow[0].contains("dotfield"));

        // the label and the name always survive, however tight it gets
        profile.label = Some("a-very-long-display-name".into());
        let tight = picker_rows(&[(&profile, text)], 40);
        assert!(tight[0].contains("account-2"));
    }

    #[test]
    fn email_shows_whole_or_not_at_all() {
        let mut profile = Profile {
            name: "account-2".into(),
            dir: PathBuf::from("/tmp/account-2"),
            exists: true,
            email: Some("account@dotfield.xyz".into()),
            plan: Some("max".into()),
            auth: Some("claude.ai".into()),
            label: Some("account".into()),
            last_used: Some(SystemTime::now()),
        };
        let text = UsageText {
            full: "5h  16% · 7d  69%  ↻4h58m".to_string(),
            compact: "5h  16% · 7d  69%".to_string(),
        };
        for width in [72, 80, 100] {
            let line = &picker_rows(&[(&profile, text.clone())], width)[0];
            assert!(
                line.contains("account@dotfield.xyz"),
                "width {width}: {line}"
            );
        }
        // narrow: the column disappears whole, never as a fragment
        for width in [40, 50, 60] {
            let line = &picker_rows(&[(&profile, text.clone())], width)[0];
            assert!(!line.contains("dotfield"), "width {width}: {line}");
            assert!(!line.contains("account@"), "width {width}: {line}");
        }

        profile.email = Some("x@y.z".into());
        let line = &picker_rows(&[(&profile, text)], 100)[0];
        assert!(line.contains("x@y.z"));
    }

    fn sample(name: &str, auth: &str, email: Option<&str>) -> Profile {
        Profile {
            name: name.into(),
            dir: PathBuf::from(format!("/tmp/{name}")),
            exists: auth != "missing",
            email: email.map(String::from),
            plan: None,
            auth: Some(auth.into()),
            label: None,
            last_used: None,
        }
    }

    #[test]
    fn accounts_that_need_a_login_sink_to_the_bottom() {
        let mut profiles = vec![
            sample("default", "logged-out", None),
            sample("work", "claude.ai", Some("work@corp.com")),
            sample("gone", "missing", None),
            sample("personal", "claude.ai", Some("me@home.com")),
            sample("stale", "unknown", None),
        ];
        sort_for_picker(&mut profiles);
        let names: Vec<&str> = profiles.iter().map(|p| p.name.as_str()).collect();
        // usable accounts keep their order, the rest follow below
        assert_eq!(names, vec!["work", "personal", "stale", "default", "gone"]);
        let first_needs = profiles.iter().position(needs_login).expect("a line");
        assert!(profiles[first_needs..].iter().all(needs_login));
    }

    #[test]
    fn signed_out_rows_say_so_in_the_usage_cell() {
        let columns = Columns::default();
        let line = picker_line(
            &sample("default", "logged-out", None),
            "5h   8%",
            "5h   8%",
            &columns,
        );
        assert!(line.contains("logged out"), "{line}");
        assert!(!line.contains("5h"), "{line}");
        let line = picker_line(
            &sample("gone", "missing", None),
            "5h   8%",
            "5h   8%",
            &columns,
        );
        assert!(line.contains("not set up"), "{line}");
        let line = picker_line(
            &sample("work", "claude.ai", Some("work@corp.com")),
            "5h   8%",
            "5h   8%",
            &columns,
        );
        assert!(line.contains("5h   8%"), "{line}");
    }

    #[test]
    fn visible_length_ignores_colour() {
        assert_eq!(visible_len("abc"), 3);
        assert_eq!(visible_len("\x1b[36mabc\x1b[0m"), 3);
        assert_eq!(visible_len("\x1b[1m\x1b[36mab\x1b[0m"), 2);
    }

    #[test]
    fn email_detection() {
        assert!(is_email("a@b.co"));
        assert!(is_email("first.last@sub.example.com"));
        assert!(!is_email("work"));
        assert!(!is_email("a@b"));
        assert!(!is_email("a@.b"));
        assert!(!is_email("a@b."));
        assert!(!is_email("a b@c.d"));
        assert!(!is_email("a@b@c.d"));
        assert!(!is_email("@b.co"));
    }

    #[test]
    fn slug_rules() {
        assert_eq!(slugify("Account@Example.com"), "account");
        assert_eq!(slugify("John Doe+tag"), "john-doe-tag");
        assert_eq!(slugify("--a__b--"), "a__b");
        assert_eq!(slugify("x".repeat(40).as_str()).len(), 24);
        assert_eq!(slugify("日本"), "");
    }

    #[test]
    fn names_from_email() {
        assert_eq!(profile_name_from_email("Me.Work@corp.com"), "me.work");
        assert_eq!(profile_name_from_email("日本@corp.com"), "account");
        assert_eq!(account_name("me@corp.com"), "me");
        assert_eq!(account_name("plain"), "plain");
    }

    #[test]
    fn plan_parsing() {
        assert_eq!(
            plan_from_account(&json!({"organizationType": "claude_max"})).as_deref(),
            Some("max")
        );
        assert_eq!(
            plan_from_account(&json!({"organizationRateLimitTier": "default_claude_pro"}))
                .as_deref(),
            Some("pro")
        );
        assert_eq!(
            plan_from_account(&json!({"seatTier": "Team_Premium"})).as_deref(),
            Some("team")
        );
        assert_eq!(plan_from_account(&json!({})), None);
    }

    #[test]
    fn ages() {
        let now = SystemTime::now();
        assert_eq!(human_age(None), "never");
        assert_eq!(human_age(Some(now)), "just now");
        assert_eq!(human_age(Some(now - Duration::from_secs(600))), "10m ago");
        assert_eq!(
            human_age(Some(now - Duration::from_secs(3 * 3600))),
            "3h ago"
        );
        assert_eq!(
            human_age(Some(now - Duration::from_secs(50 * 3600))),
            "2d ago"
        );
    }
}
