//! Per-account rate-limit usage: the 5-hour window and the weekly window.
//!
//! Claude Code answers `GET /api/oauth/usage` with `five_hour` and `seven_day`
//! (plus per-model buckets). The picker only needs those two percentages and when each
//! window resets, so this module fetches that one endpoint, keeps a small cache next to
//! the profiles, and never blocks the picker for long.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::profile;

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const TIMEOUT: Duration = Duration::from_secs(3);
/// A snapshot younger than this is reused instead of asking again, so opening the picker
/// twice in a row does not spend the endpoint's patience.
const FRESH_MS: u64 = 60_000;

/// Utilization + reset time for the two windows the picker shows.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Usage {
    pub five_hour: Option<f64>,
    pub seven_day: Option<f64>,
    /// Unix seconds, as reported by the API.
    pub five_resets: Option<i64>,
    pub seven_resets: Option<i64>,
    /// When this snapshot was read, in unix milliseconds. 0 means "unknown".
    pub fetched_at_ms: u64,
}

/// Where a row's numbers came from, so the picker can stay honest about them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    /// A fetch for this profile is still running.
    #[default]
    Loading,
    /// Numbers from this run.
    Live,
    /// Numbers from the last successful run (or from Claude Code's own cache).
    Cached,
    /// No numbers: no credentials, an expired token, or an offline machine.
    Error,
    /// `CCW_NO_USAGE=1`: no numbers will ever arrive this run.
    Off,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Slot {
    pub usage: Option<Usage>,
    pub state: State,
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

/// `CCW_NO_USAGE=1` turns every network call off; the picker then shows cached numbers only.
pub fn disabled() -> bool {
    std::env::var("CCW_NO_USAGE").is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
}

// ------------------------------------------------------------------ formatting

/// Round the way Claude Code does: whole percent, 1..99, and never a fake 0 or 100.
pub fn percent(utilization: f64) -> u32 {
    if !utilization.is_finite() || utilization <= 0.0 {
        return 0;
    }
    if utilization >= 100.0 {
        return 100;
    }
    utilization.round() as u32
}

/// "4h12m", "18h", "3d" — time until a window resets. Bounded to five characters so the
/// picker column never grows.
pub fn human_in(reset: i64) -> String {
    let secs = reset - now_secs();
    if secs <= 0 {
        return "now".into();
    }
    let min = secs / 60;
    if min < 60 {
        return format!("{min}m");
    }
    let hours = min / 60;
    let rem = min % 60;
    if hours < 10 {
        return if rem == 0 {
            format!("{hours}h")
        } else {
            format!("{hours}h{rem}m")
        };
    }
    if hours < 24 {
        return format!("{hours}h");
    }
    format!("{}d", hours / 24)
}

/// Width of the full [`cell`].
pub const CELL_WIDTH: usize = 25;
/// Width of [`cell_compact`], which drops the reset time.
pub const CELL_SHORT_WIDTH: usize = 17;

/// `5h  16% · 7d  69%` without the reset time, for narrow terminals.
pub fn cell_compact(usage: &Usage) -> String {
    let pct = |value: Option<f64>| match value {
        Some(p) => format!("{:>3}%", percent(p)),
        None => "  -".to_string(),
    };
    format!("5h {} · 7d {}", pct(usage.five_hour), pct(usage.seven_day))
}

/// Width of [`cell`] depends on which variant is in use.
pub fn cell_width(compact: bool) -> usize {
    if compact {
        CELL_SHORT_WIDTH
    } else {
        CELL_WIDTH
    }
}

/// `5h   8% · 7d  69%  ↻19m` — both windows, and the sooner reset.
pub fn cell(usage: &Usage) -> String {
    let pct = |value: Option<f64>| match value {
        Some(p) => format!("{:>3}%", percent(p)),
        None => "  -".to_string(),
    };
    let sooner = [usage.five_resets, usage.seven_resets]
        .into_iter()
        .flatten()
        .min();
    let reset = sooner
        .map(|r| format!("↻{}", human_in(r)))
        .unwrap_or_default();
    format!(
        "5h {} · 7d {}  {reset}",
        pct(usage.five_hour),
        pct(usage.seven_day)
    )
}

/// Text for a usage column: the numbers, or why they are missing. `None` means the fetch
/// has not reported back yet.
pub fn cell_for(slot: Option<&Slot>) -> String {
    let Some(slot) = slot else {
        return "checking…".into();
    };
    if let Some(usage) = slot.usage {
        return cell(&usage);
    }
    match slot.state {
        State::Loading => "checking…".into(),
        State::Off => "usage off".into(),
        State::Error => "usage n/a".into(),
        State::Cached | State::Live => cell(&Usage::default()),
    }
}

// ------------------------------------------------------------------ parsing

/// Days since 1970-01-01 for a civil date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 } as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Seconds since the epoch for an RFC 3339 timestamp. Fractional seconds are dropped.
pub fn parse_rfc3339(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    if bytes.len() < 20 {
        return None;
    }
    let num = |from: usize, len: usize| -> Option<i64> {
        let slice = bytes.get(from..from + len)?;
        let s = std::str::from_utf8(slice).ok()?;
        s.parse::<i64>().ok()
    };
    let (year, month, day) = (num(0, 4)?, num(5, 2)? as u32, num(8, 2)? as u32);
    let (hour, minute, second) = (num(11, 2)?, num(14, 2)?, num(17, 2)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let local = days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second;
    // +HH:MM / -HH:MM / Z
    let offset_at = text.rfind(['+', '-', 'Z', 'z']).filter(|i| *i >= 19);
    let offset = match offset_at.map(|i| &text[i..]) {
        Some("Z") | Some("z") | None => 0,
        Some(tail) => {
            let sign = if tail.starts_with('-') { -1 } else { 1 };
            let hours = tail.get(1..3).and_then(|s| s.parse::<i64>().ok())?;
            let minutes = tail
                .get(4..6)
                .and_then(|s| s.parse::<i64>().ok())
                .unwrap_or(0);
            sign * (hours * 3600 + minutes * 60)
        }
    };
    Some(local - offset)
}

fn number(value: Option<&Value>, key: &str) -> Option<f64> {
    value?.get(key).and_then(Value::as_f64)
}

fn reset_of(value: Option<&Value>, key: &str) -> Option<i64> {
    match value?.get(key)? {
        Value::String(s) => parse_rfc3339(s),
        Value::Number(n) => n.as_i64(),
        _ => None,
    }
}

/// Read one usage window out of the API response (or out of Claude Code's cache).
fn window(value: Option<&Value>) -> (Option<f64>, Option<i64>) {
    let Some(value) = value.filter(|v| !v.is_null()) else {
        return (None, None);
    };
    let pct = number(Some(value), "utilization");
    // The API uses resets_at, Claude Code's cache uses resetsAt.
    let reset = reset_of(Some(value), "resets_at").or_else(|| {
        value
            .get("resetsAt")
            .and_then(Value::as_i64)
            .or_else(|| reset_of(Some(value), "resetsAt"))
    });
    (pct, reset)
}

/// Parse the `/api/oauth/usage` body.
pub fn parse_usage(body: &str) -> Option<Usage> {
    let json: Value = serde_json::from_str(body).ok()?;
    let (five_hour, five_resets) = window(json.get("five_hour"));
    let (seven_day, seven_resets) = window(json.get("seven_day"));
    if five_hour.is_none() && seven_day.is_none() {
        return None;
    }
    Some(Usage {
        five_hour,
        seven_day,
        five_resets,
        seven_resets,
        fetched_at_ms: now_ms(),
    })
}

// ------------------------------------------------------------------ cache

fn cache_dir() -> PathBuf {
    profile::paths().usage.clone()
}

fn cache_path(name: &str) -> PathBuf {
    cache_dir().join(format!("{name}.json"))
}

fn to_json(usage: &Usage) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("fetchedAtMs".into(), Value::from(usage.fetched_at_ms));
    for (key, pct, reset) in [
        ("five_hour", usage.five_hour, usage.five_resets),
        ("seven_day", usage.seven_day, usage.seven_resets),
    ] {
        if let Some(pct) = pct {
            map.insert(
                key.into(),
                serde_json::json!({ "utilization": pct, "resetsAt": reset }),
            );
        }
    }
    Value::Object(map)
}

fn write_cache(name: &str, usage: &Usage) {
    let dir = cache_dir();
    if !dir.exists() && fs::create_dir_all(&dir).is_err() {
        return;
    }
    let text = match serde_json::to_string(&to_json(usage)) {
        Ok(t) => t,
        Err(_) => return,
    };
    let _ = fs::write(cache_path(name), text);
}

fn read_cache_file(name: &str) -> Option<Usage> {
    let text = fs::read_to_string(cache_path(name)).ok()?;
    let json: Value = serde_json::from_str(&text).ok()?;
    let (five_hour, five_resets) = window(json.get("five_hour"));
    let (seven_day, seven_resets) = window(json.get("seven_day"));
    if five_hour.is_none() && seven_day.is_none() {
        return None;
    }
    Some(Usage {
        five_hour,
        seven_day,
        five_resets,
        seven_resets,
        fetched_at_ms: json.get("fetchedAtMs").and_then(Value::as_u64).unwrap_or(0),
    })
}

/// Claude Code caches the same numbers in `.claude.json` (`cachedUsageUtilization`).
/// Useful when this profile has no credentials file to call the API with.
fn read_claude_cache(name: &str) -> Option<Usage> {
    let text = fs::read_to_string(profile::claude_json(name)).ok()?;
    let json: Value = serde_json::from_str(&text).ok()?;
    let block = json.get("cachedUsageUtilization")?;
    let (five_hour, five_resets) = window(block.get("five_hour"));
    let (seven_day, seven_resets) = window(block.get("seven_day"));
    if five_hour.is_none() && seven_day.is_none() {
        return None;
    }
    Some(Usage {
        five_hour,
        seven_day,
        five_resets,
        seven_resets,
        fetched_at_ms: block
            .get("fetchedAtMs")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    })
}

/// Best snapshot available without touching the network.
pub fn cached(name: &str) -> Option<Usage> {
    read_cache_file(name).or_else(|| read_claude_cache(name))
}

// ------------------------------------------------------------------ fetching

fn access_token(name: &str) -> Option<String> {
    let path = profile::profile_dir(name).join(".credentials.json");
    let text = fs::read_to_string(path).ok()?;
    let json: Value = serde_json::from_str(&text).ok()?;
    let oauth = json.get("claudeAiOauth")?;
    if let (Some(expires), Some(_)) = (
        oauth.get("expiresAt").and_then(Value::as_i64),
        oauth.get("accessToken").and_then(Value::as_str),
    ) {
        if expires / 1000 <= now_secs() {
            return None;
        }
    }
    oauth
        .get("accessToken")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .map(String::from)
}

/// Ask Claude's own usage endpoint for one profile. Authenticates with the profile's
/// OAuth access token; no token refresh, so an expired session just reports `None`.
pub fn fetch(name: &str) -> Option<Usage> {
    let token = access_token(name)?;
    let agent = ureq::AgentBuilder::new().timeout(TIMEOUT).build();
    let response = agent
        .get(USAGE_URL)
        .set("Authorization", &format!("Bearer {token}"))
        .set("anthropic-beta", "oauth-2025-04-20")
        .set("User-Agent", concat!("ccw/", env!("CARGO_PKG_VERSION")))
        .call()
        .ok()?;
    let body = response.into_string().ok()?;
    let usage = parse_usage(&body)?;
    write_cache(name, &usage);
    Some(usage)
}

/// One lookup: cached numbers first, a fetch when that is allowed.
pub fn lookup(name: &str, allow_network: bool) -> Slot {
    let cached = cached(name);
    if !allow_network {
        return match cached {
            Some(usage) => Slot {
                usage: Some(usage),
                state: State::Cached,
            },
            None => Slot::default(),
        };
    }
    match fetch(name) {
        Some(usage) => Slot {
            usage: Some(usage),
            state: State::Live,
        },
        None => match cached {
            Some(usage) => Slot {
                usage: Some(usage),
                state: State::Cached,
            },
            None => Slot {
                usage: None,
                state: State::Error,
            },
        },
    }
}

type Board = Arc<Mutex<HashMap<String, Slot>>>;

/// Fetches usage for several profiles in parallel so the picker can open immediately.
pub struct Loader {
    board: Board,
    handles: Vec<JoinHandle<()>>,
    network: bool,
}

impl Loader {
    /// Start with whatever is on disk, then fill in the live numbers in the background.
    pub fn start(names: Vec<String>) -> Loader {
        let network = !disabled();
        let idle = if network { State::Loading } else { State::Off };
        let board: Board = Arc::new(Mutex::new(HashMap::new()));
        {
            let mut map = board.lock().unwrap_or_else(|e| e.into_inner());
            for name in &names {
                let usage = cached(name);
                map.insert(
                    name.clone(),
                    Slot {
                        state: if usage.is_some() { State::Cached } else { idle },
                        usage,
                    },
                );
            }
        }
        let mut loader = Loader {
            board,
            handles: Vec::new(),
            network,
        };
        loader.reload(names, false);
        loader
    }

    /// Restart the fetch for `names`, keeping the numbers already on screen. With `force`
    /// it ignores how recent the cached snapshot is.
    pub fn reload(&mut self, names: Vec<String>, force: bool) {
        if !self.network {
            return;
        }
        let snapshot = self.snapshot();
        let now = now_ms();
        for name in names {
            if !force {
                let fresh = snapshot
                    .get(&name)
                    .and_then(|slot| slot.usage)
                    .is_some_and(|usage| {
                        usage.fetched_at_ms > 0
                            && now.saturating_sub(usage.fetched_at_ms) < FRESH_MS
                    });
                if fresh {
                    continue;
                }
            }
            if let Ok(mut map) = self.board.lock() {
                let entry = map.entry(name.clone()).or_default();
                if entry.usage.is_none() {
                    entry.state = State::Loading;
                }
            }
            let board = Arc::clone(&self.board);
            let slot_name = name;
            let handle = thread::spawn(move || {
                let slot = lookup(&slot_name, true);
                if let Ok(mut map) = board.lock() {
                    map.insert(slot_name, slot);
                }
            });
            self.handles.push(handle);
        }
        self.handles.retain(|h| !h.is_finished());
    }

    /// Copy the current numbers out so a redraw never holds the lock.
    pub fn snapshot(&self) -> HashMap<String, Slot> {
        self.board.lock().map(|map| map.clone()).unwrap_or_default()
    }

    /// True while at least one fetch is still running.
    pub fn pending(&self) -> bool {
        self.handles.iter().any(|h| !h.is_finished())
    }

    /// Wait until every fetch is done, or `limit` passes.
    pub fn settle(&mut self, limit: Duration) {
        let start = std::time::Instant::now();
        while self.pending() && start.elapsed() < limit {
            thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Loader {
    fn drop(&mut self) {
        // Short timeouts bound these threads; do not hold up the launch waiting for them.
        self.handles.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_days() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11017);
        assert_eq!(days_from_civil(2026, 11, 3), 20760);
    }

    #[test]
    fn rfc3339_parsing() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339("2026-11-03T05:09:59Z"), Some(1_793_682_599));
        assert_eq!(
            parse_rfc3339("2026-11-03T05:09:59.906752+00:00"),
            Some(1_793_682_599)
        );
        assert_eq!(
            parse_rfc3339("2026-11-03T14:09:59+09:00"),
            Some(1_793_682_599)
        );
        assert_eq!(
            parse_rfc3339("2026-11-03T00:00:00-05:00"),
            Some(1_793_682_000)
        );
        assert_eq!(parse_rfc3339("nope"), None);
    }

    #[test]
    fn usage_body_parsing() {
        let body = r#"{
            "five_hour": {"utilization": 8.0, "resets_at": "2026-11-03T05:09:59.906752+00:00"},
            "seven_day": {"utilization": 69.0, "resets_at": "2026-11-03T22:59:59.906779+00:00"},
            "seven_day_opus": null
        }"#;
        let usage = parse_usage(body).expect("parses");
        assert_eq!(usage.five_hour, Some(8.0));
        assert_eq!(usage.seven_day, Some(69.0));
        assert_eq!(usage.five_resets, Some(1_793_682_599));
        assert!(usage.fetched_at_ms > 0);
    }

    #[test]
    fn cache_shape_round_trip() {
        let usage = Usage {
            five_hour: Some(8.0),
            seven_day: Some(69.0),
            five_resets: Some(1_793_682_599),
            seven_resets: Some(1_793_750_999),
            fetched_at_ms: 42,
        };
        let text = serde_json::to_string(&to_json(&usage)).expect("json");
        let json: Value = serde_json::from_str(&text).expect("json");
        let (pct, reset) = window(json.get("five_hour"));
        assert_eq!((pct, reset), (Some(8.0), Some(1_793_682_599)));
        let (pct, reset) = window(json.get("seven_day"));
        assert_eq!((pct, reset), (Some(69.0), Some(1_793_750_999)));
    }

    #[test]
    fn claude_cache_uses_camel_case_reset() {
        let json: Value = serde_json::from_str(
            r#"{"five_hour":{"utilization":12.0,"resetsAt":1793682599},
                "seven_day":{"utilization":55.0,"resetsAt":1793750399}}"#,
        )
        .expect("json");
        let (pct, reset) = window(json.get("five_hour"));
        assert_eq!((pct, reset), (Some(12.0), Some(1_793_682_599)));
    }

    #[test]
    fn empty_body_is_none() {
        assert!(parse_usage("{}").is_none());
        assert!(parse_usage("not json").is_none());
        assert!(parse_usage(r#"{"five_hour":null,"seven_day":null}"#).is_none());
    }

    #[test]
    fn percentages() {
        assert_eq!(percent(0.0), 0);
        assert_eq!(percent(8.0), 8);
        assert_eq!(percent(8.4), 8);
        assert_eq!(percent(8.6), 9);
        assert_eq!(percent(100.0), 100);
        assert_eq!(percent(-3.0), 0);
        assert_eq!(percent(f64::NAN), 0);
    }

    #[test]
    fn countdowns() {
        let now = now_secs();
        assert_eq!(human_in(now - 10), "now");
        assert_eq!(human_in(now + 60), "1m");
        assert_eq!(human_in(now + 3600), "1h");
        assert_eq!(human_in(now + 3600 + 12 * 60), "1h12m");
        assert_eq!(human_in(now + 12 * 3600 + 5 * 60), "12h");
        assert_eq!(human_in(now + 3 * 86_400), "3d");
        for secs in [60, 3599, 3600, 35999, 36000, 86_399, 86_400, 9_999_999] {
            assert!(human_in(now + secs).chars().count() <= 5);
        }
    }

    #[test]
    fn cells() {
        let now = now_secs();
        let usage = Usage {
            five_hour: Some(8.0),
            seven_day: Some(69.0),
            five_resets: Some(now + 19 * 60),
            seven_resets: Some(now + 18 * 3600),
            fetched_at_ms: 1,
        };
        assert_eq!(cell(&usage), "5h   8% · 7d  69%  ↻19m");
        assert!(cell(&usage).chars().count() <= CELL_WIDTH);

        let full = Usage {
            five_hour: Some(100.0),
            seven_day: Some(100.0),
            five_resets: Some(now + 9 * 3600 + 59 * 60),
            seven_resets: Some(now + 6 * 86_400),
            fetched_at_ms: 1,
        };
        assert_eq!(cell(&full), "5h 100% · 7d 100%  ↻9h59m");
        assert_eq!(cell(&full).chars().count(), CELL_WIDTH);

        assert_eq!(cell(&Usage::default()), "5h   - · 7d   -  ");

        // whatever the numbers, the column keeps its width once padded
        for sample in [usage, full, Usage::default()] {
            assert_eq!(
                crate::style::pad_end(&cell(&sample), CELL_WIDTH)
                    .chars()
                    .count(),
                CELL_WIDTH
            );
        }
    }
}
