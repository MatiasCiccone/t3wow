//! What the companion writes for the addon: JSON values as Lua 5.1 literals, and T3's
//! markdown as WoW FontString markup.

use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;

const RESERVED: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "if", "in", "local", "nil", "not", "or",
    "repeat", "return", "then", "true", "until", "while", "goto",
];

pub fn lua_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 32 || c as u32 == 127 => out.push_str(&format!("\\{:03}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn is_identifier(key: &str) -> bool {
    let mut chars = key.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !RESERVED.contains(&key)
}

/// A JSON value as a Lua literal. Arrays become sequences; null fields are dropped.
pub fn to_lua(value: &Value) -> String {
    match value {
        Value::Null => "nil".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => {
            if n.is_i64() || n.is_u64() {
                n.to_string()
            } else {
                match n.as_f64() {
                    Some(f) if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e15 => format!("{}", f as i64),
                    Some(f) if f.is_finite() => f.to_string(),
                    _ => "0".into(),
                }
            }
        }
        Value::String(s) => lua_string(s),
        Value::Array(items) => format!("{{{}}}", items.iter().map(to_lua).collect::<Vec<_>>().join(",")),
        Value::Object(fields) => {
            let parts: Vec<String> = fields
                .iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| {
                    let name = if is_identifier(k) { k.clone() } else { format!("[{}]", lua_string(k)) };
                    format!("{name}={}", to_lua(v))
                })
                .collect();
            format!("{{{}}}", parts.join(","))
        }
    }
}

/// Text for a one-line field: pipes escaped for WoW, whitespace collapsed, at most `max` chars.
pub fn plain(text: &str, max: usize) -> String {
    let collapsed = text.replace('|', "||").split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(max).collect()
}

const CODE: &str = "|cffc8c8a0";
const HEAD: &str = "|cffffd100";

struct Patterns {
    fence: Regex,
    image: Regex,
    link: Regex,
    bold: Regex,
    under: Regex,
    code: Regex,
    bullet: Regex,
    heading: Regex,
    gaps: Regex,
}

fn patterns() -> &'static Patterns {
    static PATTERNS: OnceLock<Patterns> = OnceLock::new();
    PATTERNS.get_or_init(|| Patterns {
        fence: Regex::new(r"^\s*```").unwrap(),
        image: Regex::new(r"!\[([^\]]*)\]\([^)]*\)").unwrap(),
        link: Regex::new(r"\[([^\]]+)\]\(([^)]+)\)").unwrap(),
        bold: Regex::new(r"\*\*([^*]+)\*\*").unwrap(),
        under: Regex::new(r"__([^_]+)__").unwrap(),
        code: Regex::new(r"`([^`]+)`").unwrap(),
        bullet: Regex::new(r"^(\s*)[-*] ").unwrap(),
        heading: Regex::new(r"^#{1,6}\s+(.*)$").unwrap(),
        gaps: Regex::new(r"\n{3,}").unwrap(),
    })
}

/// Markdown to WoW FontString markup. `|` starts an escape sequence in WoW text, so
/// literal pipes are doubled before any color codes are added.
pub fn wow_text(markdown: &str, max_length: usize) -> String {
    let p = patterns();
    let mut text = markdown.replace("\r\n", "\n").replace('\r', "\n");
    if text.chars().count() > max_length {
        text = text.chars().take(max_length).collect::<String>() + "\n…";
    }
    let text = text.replace('|', "||");
    let mut lines = Vec::new();
    let mut in_fence = false;
    for line in text.split('\n') {
        if p.fence.is_match(line) {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            lines.push(format!("{CODE}{line}|r"));
            continue;
        }
        let l = p.image.replace_all(line, "[$1]");
        let l = p.link.replace_all(&l, "$1");
        let l = p.bold.replace_all(&l, "$1");
        let l = p.under.replace_all(&l, "$1");
        let l = p.code.replace_all(&l, format!("{CODE}$1|r").as_str());
        let l = p.bullet.replacen(&l, 1, "$1• ").into_owned();
        let l = match p.heading.captures(&l) {
            Some(heading) => format!("{HEAD}{}|r", &heading[1]),
            None => l,
        };
        lines.push(l);
    }
    p.gaps.replace_all(&lines.join("\n"), "\n\n").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_inbox_is_written_exactly_like_before() {
        let inbox: Value = serde_json::from_str(include_str!("../tests/fixtures/inbox.json")).unwrap();
        let expected = include_str!("../tests/fixtures/inbox.lua").replace("\r\n", "\n");
        assert_eq!(format!("T3WoW_Inbox = {}\n", to_lua(&inbox)), expected);
    }

    #[test]
    fn markdown_becomes_the_same_wow_text() {
        let cases: Value = serde_json::from_str(include_str!("../tests/fixtures/wowtext.json")).unwrap();
        for case in cases.as_array().unwrap() {
            assert_eq!(wow_text(case["input"].as_str().unwrap(), 6000), case["output"].as_str().unwrap());
        }
    }

    #[test]
    fn plain_text_is_one_short_line() {
        assert_eq!(plain("  a |\n b\t c  ", 200), "a || b c");
        assert_eq!(plain("abcdef", 3), "abc");
    }
}
