use std::sync::OnceLock;

/// ANSI colors, all empty when NO_COLOR is set.
pub struct Colors {
    pub reset: &'static str,
    pub green: &'static str,
    pub yellow: &'static str,
    pub cyan: &'static str,
    pub dim: &'static str,
    pub bold: &'static str,
    pub show_cursor: &'static str,
}

pub fn c() -> &'static Colors {
    static COLORS: OnceLock<Colors> = OnceLock::new();
    COLORS.get_or_init(|| {
        if std::env::var_os("NO_COLOR").is_some() {
            Colors {
                reset: "",
                green: "",
                yellow: "",
                cyan: "",
                dim: "",
                bold: "",
                show_cursor: "",
            }
        } else {
            Colors {
                reset: "\x1b[0m",
                green: "\x1b[32m",
                yellow: "\x1b[33m",
                cyan: "\x1b[36m",
                dim: "\x1b[90m",
                bold: "\x1b[1m",
                show_cursor: "\x1b[?25h",
            }
        }
    })
}

/// Turn on ANSI escape handling for older Windows consoles. No-op elsewhere.
pub fn init_terminal() {
    #[cfg(windows)]
    {
        let _ = crossterm::ansi_support::supports_ansi();
    }
}

/// Terminal width in columns. `COLUMNS` wins when set, so piped output can be shaped too;
/// otherwise ask the terminal, and fall back to 80.
pub fn width() -> usize {
    if let Some(cols) = std::env::var("COLUMNS")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|w| *w >= 20)
    {
        return cols;
    }
    crossterm::terminal::size()
        .map(|(w, _)| w as usize)
        .ok()
        .filter(|w| *w >= 20)
        .unwrap_or(80)
}

/// Truncate to `width` characters and pad with spaces.
pub fn pad(s: &str, width: usize) -> String {
    let mut out: String = s.chars().take(width).collect();
    let len = out.chars().count();
    out.extend(std::iter::repeat(' ').take(width - len));
    out
}

/// Pad with spaces without truncating.
pub fn pad_end(s: &str, width: usize) -> String {
    let len = s.chars().count();
    if len >= width {
        return s.to_string();
    }
    let mut out = s.to_string();
    out.extend(std::iter::repeat(' ').take(width - len));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pad_truncates_and_fills() {
        assert_eq!(pad("abc", 5), "abc  ");
        assert_eq!(pad("abcdef", 3), "abc");
        assert_eq!(pad("日本語", 4), "日本語 ");
    }

    #[test]
    fn pad_end_keeps_long_text() {
        assert_eq!(pad_end("abcdef", 3), "abcdef");
        assert_eq!(pad_end("ab", 3), "ab ");
    }
}
