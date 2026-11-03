//! Launch options: model and effort level, remembered per profile.

use std::fs;

use crate::profile;

pub const EFFORT_LEVELS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

/// Rows in the model picker, in display order. `default` sends no `--model` at all.
pub const MODELS: [(&str, &str); 5] = [
    ("default", "no --model, let Claude Code pick"),
    ("opus", "alias for the newest Opus"),
    ("sonnet", "alias for the newest Sonnet"),
    ("haiku", "alias for the newest Haiku"),
    ("fable", "alias for the newest Fable"),
];

pub fn effort_note(level: &str) -> &'static str {
    match level {
        "low" => "fastest answer, least thinking",
        "medium" => "less than the model default",
        "high" => "the usual default",
        "xhigh" => "more thinking than default",
        "max" => "the most thinking the model allows",
        _ => "",
    }
}

/// What to pass to claude on top of the user's own arguments.
#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub model: Option<String>,
    pub effort: Option<String>,
}

impl Choice {
    pub fn is_default(&self) -> bool {
        self.model.is_none() && self.effort.is_none()
    }

    /// `--model X --effort Y`, or nothing at all.
    pub fn args(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(m) = &self.model {
            out.push("--model".into());
            out.push(m.clone());
        }
        if let Some(e) = &self.effort {
            out.push("--effort".into());
            out.push(e.clone());
        }
        out
    }

    /// Short form for one-line messages: "opus, high effort".
    pub fn summary(&self) -> String {
        if self.is_default() {
            return "claude defaults".into();
        }
        let model = self.model.clone().unwrap_or_else(|| "default model".into());
        match &self.effort {
            Some(e) => format!("{model}, {e} effort"),
            None => model,
        }
    }
}

/// Documented levels, any case. `default`, `none` and an empty string mean "omit the flag".
pub fn parse_effort(raw: &str) -> Result<Option<String>, String> {
    let value = raw.trim().to_ascii_lowercase();
    if value.is_empty() || value == "default" || value == "none" {
        return Ok(None);
    }
    EFFORT_LEVELS
        .iter()
        .find(|level| **level == value)
        .map(|level| Some((*level).to_string()))
        .ok_or_else(|| {
            format!(
                "\"{raw}\" is not an effort level ({})",
                EFFORT_LEVELS.join(", ")
            )
        })
}

/// Any alias or full model ID. `default` and an empty string mean "omit the flag".
pub fn parse_model(raw: &str) -> Option<String> {
    let value = raw.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("default") {
        return None;
    }
    Some(value.to_string())
}

fn prefs_path(name: &str) -> std::path::PathBuf {
    profile::paths().prefs.join(name)
}

/// Remembered choice for a profile. Missing or unreadable files mean "claude defaults".
pub fn read(name: &str) -> Choice {
    let mut choice = Choice::default();
    let Ok(text) = fs::read_to_string(prefs_path(name)) else {
        return choice;
    };
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "model" => choice.model = parse_model(value),
            "effort" => choice.effort = parse_effort(value).ok().flatten(),
            _ => {}
        }
    }
    choice
}

/// Save the choice. A default choice removes the file again.
pub fn write(name: &str, choice: &Choice) -> std::io::Result<()> {
    let file = prefs_path(name);
    if choice.is_default() {
        let _ = fs::remove_file(file);
        return Ok(());
    }
    fs::create_dir_all(&profile::paths().prefs)?;
    let mut text = String::new();
    if let Some(model) = &choice.model {
        text.push_str(&format!("model={model}\n"));
    }
    if let Some(effort) = &choice.effort {
        text.push_str(&format!("effort={effort}\n"));
    }
    fs::write(file, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effort_levels() {
        assert_eq!(parse_effort("high").unwrap().as_deref(), Some("high"));
        assert_eq!(parse_effort("XHIGH").unwrap().as_deref(), Some("xhigh"));
        assert_eq!(parse_effort(" max ").unwrap().as_deref(), Some("max"));
        assert_eq!(parse_effort("default").unwrap(), None);
        assert_eq!(parse_effort("").unwrap(), None);
        assert!(parse_effort("turbo").is_err());
    }

    #[test]
    fn model_values() {
        assert_eq!(parse_model("opus").as_deref(), Some("opus"));
        assert_eq!(
            parse_model("claude-opus-4-8").as_deref(),
            Some("claude-opus-4-8")
        );
        assert_eq!(parse_model("default"), None);
        assert_eq!(parse_model("  "), None);
    }

    #[test]
    fn args_shape() {
        let choice = Choice {
            model: Some("opus".into()),
            effort: Some("high".into()),
        };
        assert_eq!(
            args_of(&choice),
            vec![
                "--model".to_string(),
                "opus".into(),
                "--effort".into(),
                "high".into()
            ]
        );

        let only_effort = Choice {
            model: None,
            effort: Some("low".into()),
        };
        assert_eq!(
            args_of(&only_effort),
            vec!["--effort".to_string(), "low".into()]
        );
        assert!(Choice::default().args().is_empty());
        assert!(Choice::default().is_default());
    }

    #[test]
    fn summaries() {
        assert_eq!(Choice::default().summary(), "claude defaults");
        let choice = Choice {
            model: Some("opus".into()),
            effort: Some("high".into()),
        };
        assert_eq!(choice.summary(), "opus, high effort");
        let bare = Choice {
            model: Some("sonnet".into()),
            effort: None,
        };
        assert_eq!(bare.summary(), "sonnet");
    }

    #[test]
    fn every_level_has_a_note() {
        for level in EFFORT_LEVELS {
            assert!(!effort_note(level).is_empty(), "{level}");
        }
    }

    fn args_of(choice: &Choice) -> Vec<String> {
        choice.args()
    }
}
