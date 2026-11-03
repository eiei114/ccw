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

/// Column widths of one picker row, so every row lines up.
pub const LABEL_COL: usize = 20;
pub const ACCOUNT_COL: usize = 32;

/// One row of the picker: the account on the left, its rate-limit usage on the right,
/// then the profile directory and the remaining details in dim text.
pub fn picker_line(p: &Profile, usage: &str) -> String {
    let c = c();
    let shown = p
        .label
        .clone()
        .or_else(|| p.email.as_deref().map(|e| account_name(e).to_string()))
        .unwrap_or_else(|| p.name.clone());
    let label = pad(&shown, LABEL_COL);
    let usage = pad(usage, crate::usage::CELL_WIDTH);
    let tail = format!("{}{}{}", c.dim, p.name, c.reset);

    if !p.exists {
        return format!(
            "{label} {}{}{} {usage}  {}(new — press a){}",
            c.dim,
            pad("", ACCOUNT_COL),
            c.reset,
            c.dim,
            c.reset
        );
    }
    if p.auth.as_deref() == Some("logged-out") {
        return format!(
            "{label} {}{}{} {usage}  {tail}",
            c.yellow,
            pad("logged out", ACCOUNT_COL),
            c.reset
        );
    }

    let who = pad(
        p.email.as_deref().or(p.auth.as_deref()).unwrap_or("?"),
        ACCOUNT_COL,
    );
    let plan = p
        .plan
        .as_deref()
        .map(|pl| format!(" ({pl})"))
        .unwrap_or_default();
    let age = if p.last_used.is_some() {
        format!("  {}", human_age(p.last_used))
    } else {
        String::new()
    };
    format!(
        "{label} {}{who}{} {usage}  {tail}{}{plan}{age}{}",
        c.cyan, c.reset, c.dim, c.reset
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
