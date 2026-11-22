//! What the companion writes for the addon: JSON values as Lua 5.1 literals, and T3's
//! markdown as WoW FontString markup.

use regex::Regex;
use serde_json::{json, Value};
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

/// Which font draws a run of text: the window's text font (Inter, which ships with the
/// addon) or JetBrains Mono, which draws code, tables and previews and has a few more marks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Font {
    Text,
    Mono,
}

/// Marks JetBrains Mono has that Inter lacks: in code they stay as written.
const MONO_HAS: &str = "➜➔➝➞⇨⚠✕▸⚡";

/// A stand-in for a mark agents write that `font` doesn't have: one both fonts have, as
/// close as there is. None when the font has it, or there is no stand-in.
fn stand_in(c: char, font: Font) -> Option<&'static str> {
    if font == Font::Mono && MONO_HAS.contains(c) {
        return None;
    }
    Some(match c {
        '⟶' | '➜' | '➔' | '➡' | '➝' | '➞' | '⮕' | '⇨' => "→",
        '⟵' | '⬅' | '⇦' => "←",
        '⟷' | '⬌' => "↔",
        '⬆' => "↑",
        '⬇' | '⇩' => "↓",
        '⟹' => "⇒",
        '⟸' => "⇐",
        '⟺' => "⇔",
        '✅' | '✔' | '☑' => "✓",
        '❌' | '✘' | '✖' | '❎' | '✕' => "✗",
        '⚠' => "△",
        // Not "!": with a number after it, the game reads one as a merge request's (!157).
        '❗' | '❕' => "‼",
        '❓' | '❔' => "?",
        '⭐' | '★' => "◆",
        '☆' => "◇",
        '➕' => "+",
        '➖' => "-",
        '➗' => "÷",
        '☐' | '◻' | '◽' | '⬜' => "□",
        '☒' | '◼' | '◾' | '⬛' => "■",
        'ℹ' => "i",
        '▸' => "►",
        '🔴' | '🟠' | '🟡' | '🟢' | '🔵' | '🟣' | '🟤' | '⚫' => "●",
        '⚪' => "○",
        _ => return None,
    })
}

/// A picture character `font` doesn't have, with no stand-in: every emoji in the emoji
/// blocks past the first plane, and the common ones among the symbols.
fn emoji(c: char, font: Font) -> bool {
    if font == Font::Mono && MONO_HAS.contains(c) {
        return false;
    }
    ('\u{1F000}'..='\u{1FAFF}').contains(&c) || "✨⚡❤⏳⌛⏰⏱☕✏⛔⭕♻☀✋✌☝✊✍✈❄✂✉☁☔⚓⚔⚖⌚⏩⏪⏭⏸⏹⏺⏲⚙♠♣♥♦✦✧✳✴❇❣⤴⤵".contains(c)
}

/// A symbol written as an emoji (followed by U+FE0F) that `font` doesn't have: the
/// symbol and dingbat blocks, where Inter has only ✓ and ✗ (✉️, ☎️, ⚕️, …).
fn symbol_emoji(c: char, font: Font) -> bool {
    (('\u{2600}'..='\u{27BF}').contains(&c) || ('\u{2B00}'..='\u{2BFF}').contains(&c))
        && !"✓✗".contains(c)
        && !(font == Font::Mono && MONO_HAS.contains(c))
}

/// What styles an emoji or ties its sequence together, and draws nothing in the game: the
/// text and emoji selectors, the keycap, a subdivision flag's tags.
fn emoji_glue(c: char) -> bool {
    matches!(c, '\u{FE0E}' | '\u{FE0F}' | '\u{20E3}' | '\u{E0020}'..='\u{E007F}')
}

/// Whether the end of `out` starts a run of text: nothing yet, a space or a line break, a
/// color code, or the start of inline code.
fn run_starts(out: &str) -> bool {
    let n = out.len();
    let color = n >= 10
        && out.is_char_boundary(n - 10)
        && out[n - 10..].starts_with("|c")
        && out[n - 8..].bytes().all(|b| b.is_ascii_hexdigit())
        && !out[..n - 10].ends_with('|');
    out.is_empty() || out.ends_with([' ', '\t', '\n', CODE_OPEN, '"', '“', '‘', '(', '[']) || color
}

/// Game text (T3's markdown already made WoW markup) with each mark `font` doesn't have put
/// as one it does (see `stand_in`), and the emoji it can't draw left out: the game draws an
/// empty box for a character its font lacks. In `Font::Text`, inline code (between CODE_OPEN
/// and CODE_CLOSE) is JetBrains Mono's. An emoji goes with its whole sequence (joined people,
/// skin tones, flags). In text, one that starts a run takes the spaces after it, and one that
/// ends a sentence the space before it: "🚀 Shipped" reads "Shipped", "Done 🎉." "Done.". In
/// code it leaves one space, so columns stay put. Text left with nothing stays as it was.
pub fn drawable(text: &str, font: Font) -> String {
    if text.is_ascii() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut run = font;
    // Inside a marked thread link or model reference (LINK_MARK, MODEL_MARK): the bridge
    // looks it up as written, so it stays so.
    let mut verbatim = None;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if verbatim.is_some() || c == LINK_MARK || c == MODEL_MARK {
            verbatim = match verbatim {
                Some(mark) if mark == c => None,
                None => Some(c),
                open => open,
            };
            out.push(c);
        } else if font == Font::Text && (c == CODE_OPEN || c == CODE_CLOSE) {
            run = if c == CODE_OPEN { Font::Mono } else { Font::Text };
            out.push(c);
        } else if let Some(mark) = stand_in(c, run) {
            out.push_str(mark);
            while chars.next_if(|&n| emoji_glue(n)).is_some() {}
        } else if emoji(c, run)
            || (chars.peek() == Some(&'\u{FE0F}') && symbol_emoji(c, run))
            // A keycap whose sign would read as markdown or a pull request's number (#️⃣).
            || ((c == '#' || c == '*') && chars.peek().is_some_and(|&n| emoji_glue(n)))
        {
            // The rest of its sequence: what styles it, and whatever a joiner ties to it (not
            // markup: a joiner left dangling at the end of a run). In code, each emoji of a
            // row leaves a space of its own.
            let mut emojis = 1;
            loop {
                if chars.next_if(|&n| emoji_glue(n)).is_some() {
                    continue;
                }
                if chars.next_if(|&n| emoji(n, run) && stand_in(n, run).is_none()).is_some() {
                    emojis += 1;
                    continue;
                }
                if chars.next_if_eq(&'\u{200D}').is_some() {
                    chars.next_if(|&n| !n.is_ascii() && !n.is_alphanumeric());
                    continue;
                }
                break;
            }
            // Whether what follows ends the sentence or the run: punctuation, a line break, the
            // end, a color's end (|r) or inline code's.
            let mut ahead = chars.clone();
            let ends = match ahead.next() {
                None => true,
                Some('|') => ahead.next() == Some('r'),
                Some(n) => ".,;:!?)]\n\"”’'".contains(n) || n == CODE_CLOSE,
            };
            if run == Font::Mono {
                out.push_str(&" ".repeat(emojis));
            } else if ends {
                while out.ends_with(' ') {
                    out.pop();
                }
            } else if run_starts(&out) {
                while chars.next_if(|&n| n == ' ' || n == '\t').is_some() {}
            }
        } else if !emoji_glue(c) {
            out.push(c);
        }
    }
    if blank(&out) && !blank(text) {
        return text.to_string();
    }
    out
}

/// Whether game text shows nothing: only spaces, color codes and the companion's marks.
fn blank(text: &str) -> bool {
    patterns().markup.replace_all(text, "").trim().is_empty()
}

/// Text for a one-line field: pipes escaped for WoW, whitespace collapsed, at most `max` chars.
pub fn plain(text: &str, max: usize) -> String {
    let collapsed = text.replace('|', "||").split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(max).collect()
}

/// A question option's preview (a mockup or a snippet), for the game's monospace font: its
/// lines and indents kept, a code fence's ``` lines dropped, cut to `max_lines` lines and
/// `max` chars (with "…"), pipes escaped for WoW.
pub fn preview_text(text: &str, max: usize, max_lines: usize) -> String {
    let fence = &patterns().fence;
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = text.split('\n').filter(|l| !fence.is_match(l)).map(str::trim_end).collect();
    let lines = lines.join("\n");
    let lines = lines.trim_matches('\n');
    let mut kept: String = lines.split('\n').take(max_lines).collect::<Vec<_>>().join("\n").chars().take(max).collect();
    if kept.len() < lines.len() {
        kept.push('…');
    }
    drawable(&kept.replace('|', "||"), Font::Mono)
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
    delimiter: Regex,
    br: Regex,
    span: Regex,
    thread: Regex,
    markup: Regex,
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
        delimiter: Regex::new(r"^\s*\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?\s*$").unwrap(),
        br: Regex::new(r"(?i)<br\s*/?>").unwrap(),
        span: Regex::new(r"\x03(\d+)\x03").unwrap(),
        // What draws nothing in game text: color codes and the companion's marks.
        markup: Regex::new(r"\|c[0-9a-fA-F]{8}|\|r|[\x01-\x05]").unwrap(),
        // A T3 thread link: <t3-thread://v2/{env}/{thread}> as T3's tools write it, or bare
        // (its last character not punctuation that ends a sentence).
        thread: Regex::new(r#"<t3-thread://v2/([^/\s<>]+)/([^\s<>]+)>|t3-thread://v2/([^/\s<>]+)/([^\s<>()\[\]]*[^\s<>()\[\].,;:!?'"])"#).unwrap(),
    })
}

/// One line of prose (its pipes already doubled) as WoW markup: links, emphasis and bullets
/// simplified, a heading in gold. Inline code goes through `code` ("$1" is the code).
fn prose_line(line: &str, code: &str) -> String {
    let p = patterns();
    let l = inline(line, code);
    let l = p.bullet.replacen(&l, 1, "$1• ").into_owned();
    match p.heading.captures(&l) {
        Some(heading) => color_runs(&heading[1], HEAD),
        None => l,
    }
}

/// Links and emphasis simplified, inline code through `code` ("$1" is the code). Code is
/// set aside first (as \u{3}n\u{3}), so `__init__` or a link in it stays as written.
fn inline(line: &str, code: &str) -> String {
    let p = patterns();
    let mut spans: Vec<String> = Vec::new();
    let l = p.code.replace_all(line, |c: &regex::Captures| {
        spans.push(c[1].to_string());
        format!("\u{3}{}\u{3}", spans.len() - 1)
    });
    let l = p.image.replace_all(&l, "[$1]");
    let l = p.link.replace_all(&l, "$1");
    let l = p.thread.replace_all(&l, |c: &regex::Captures| {
        let (env, thread) = match (c.get(1), c.get(2)) {
            (Some(env), Some(thread)) => (env.as_str(), thread.as_str()),
            _ => (&c[3], &c[4]),
        };
        format!("{LINK_MARK}{env}/{thread}{LINK_MARK}")
    });
    let l = p.bold.replace_all(&l, "$1");
    let l = p.under.replace_all(&l, "$1");
    let l = mark_models(&l);
    let l = p.span.replace_all(&l, |c: &regex::Captures| match c[1].parse::<usize>().ok().and_then(|n| spans.get(n)) {
        Some(span) => code.replace("$1", span),
        None => c[0].to_string(),
    });
    l.into_owned()
}

/// Text in a color, each run between inline code marks colored on its own, so the addon can
/// draw the code apart without cutting a color code in two.
fn color_runs(text: &str, color: &str) -> String {
    let mut out = String::new();
    let mut in_code = false;
    for (n, run) in text.split([CODE_OPEN, CODE_CLOSE]).enumerate() {
        if n > 0 {
            out.push(if in_code { CODE_CLOSE } else { CODE_OPEN });
            in_code = !in_code;
        }
        if in_code || run.is_empty() {
            out.push_str(run);
        } else {
            out.push_str(&format!("{color}{run}|r"));
        }
    }
    out
}

/// Markdown to WoW FontString markup, for the game to draw: as `wow_markup`, with the marks
/// and emoji its fonts can't draw put as ones they can, or left out (`drawable`).
pub fn wow_text(markdown: &str, max_length: usize) -> String {
    prose_drawable(&wow_markup(markdown, max_length))
}

/// Prose made drawable (`drawable` in the text font), without the empty lines an emoji
/// alone on its line leaves.
fn prose_drawable(text: &str) -> String {
    let out = drawable(text, Font::Text);
    if out == text { out } else { patterns().gaps.replace_all(&out, "\n\n").trim().to_string() }
}

/// Markdown to WoW FontString markup, every character as T3 has it (for copying). `|` starts
/// an escape sequence in WoW text, so literal pipes are doubled before any color codes are
/// added.
pub fn wow_markup(markdown: &str, max_length: usize) -> String {
    let p = patterns();
    let mut text = markdown.replace("\r\n", "\n").replace('\r', "\n").replace([LINK_MARK, MODEL_MARK], "");
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
        lines.push(prose_line(line, &format!("{CODE}$1|r")));
    }
    p.gaps.replace_all(&lines.join("\n"), "\n\n").trim().to_string()
}

/// Marks around inline code in a prose segment: the addon draws what is between them in its
/// monospace font. T3's text can't hold them: they are taken out of it first.
pub const CODE_OPEN: char = '\u{1}';
pub const CODE_CLOSE: char = '\u{2}';

/// Marks around a T3 thread link's "{env}/{thread}" in prose and table cells: the bridge
/// puts the thread's name in its place (`name_thread_links`), since a raw link means
/// nothing in the game. T3's text can't hold them: they are taken out of it first.
pub const LINK_MARK: char = '\u{4}';

/// Text with each marked thread link (see LINK_MARK) put as `name` says for its
/// environment and thread.
pub fn name_thread_links(text: &str, name: impl Fn(&str, &str) -> String) -> String {
    if !text.contains(LINK_MARK) {
        return text.to_string();
    }
    let mut out = String::new();
    for (n, part) in text.split(LINK_MARK).enumerate() {
        if n % 2 == 0 {
            out.push_str(part);
        } else {
            let (env, thread) = part.split_once('/').unwrap_or(("", part));
            out.push_str(&name(env, thread));
        }
    }
    out
}

/// Marks around a `model:<instance>/<model>` reference in prose and table cells, as T3 0.1.8
/// writes one to name a model for the agent: the bridge puts the model's name in its place
/// when the machine has that provider and model (`name_model_refs`), else the reference as
/// written. T3's text can't hold them: they are taken out of it first.
pub const MODEL_MARK: char = '\u{5}';

/// The text with each `model:<instance>/<model>` reference marked (see MODEL_MARK), found as
/// T3's composer-model-mentions finds them: at the start or after a space, comma, bracket or
/// quote; an instance id as T3 makes them; a model running to a space, comma or quote,
/// without the sentence punctuation or unbalanced closing bracket that ends it.
fn mark_models(text: &str) -> String {
    const PREFIX: &str = "model:";
    if !text.contains(PREFIX) {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len() + 8);
    let (mut rest, mut from) = (0, 0);
    while let Some(found) = text[from..].find(PREFIX) {
        let start = from + found;
        from = start + PREFIX.len();
        let starts = text[..start].chars().next_back().is_none_or(|c| c.is_whitespace() || ",([\"'".contains(c));
        let after = &text[from..];
        let instance = after
            .char_indices()
            .take_while(|&(i, c)| if i == 0 { c.is_ascii_alphabetic() } else { c.is_ascii_alphanumeric() || c == '_' || c == '-' })
            .count();
        if !starts || instance == 0 || !after[instance..].starts_with('/') {
            continue;
        }
        let slug_from = instance + 1;
        let slug_len: usize = after[slug_from..]
            .chars()
            .take_while(|&c| !c.is_whitespace() && !c.is_control() && !",\"'".contains(c))
            .map(char::len_utf8)
            .sum();
        let slug = trim_model_slug(&after[slug_from..slug_from + slug_len]);
        if slug.is_empty() {
            continue;
        }
        let end = from + slug_from + slug.len();
        out.push_str(&text[rest..start]);
        out.push(MODEL_MARK);
        out.push_str(&text[start..end]);
        out.push(MODEL_MARK);
        (rest, from) = (end, end);
    }
    out.push_str(&text[rest..]);
    out
}

/// A model slug without what ends the sentence around it: `.;:!?`, or a `)` or `]` it
/// doesn't open itself (`foo(bar)` keeps its own).
fn trim_model_slug(slug: &str) -> &str {
    let mut trimmed = slug;
    while let Some(last) = trimmed.chars().next_back() {
        let opening = match last {
            ')' => Some('('),
            ']' => Some('['),
            _ => None,
        };
        let unbalanced = opening.is_some_and(|o| trimmed.matches(o).count() < trimmed.matches(last).count());
        if !".;:!?".contains(last) && !unbalanced {
            break;
        }
        trimmed = &trimmed[..trimmed.len() - last.len_utf8()];
    }
    trimmed
}

/// Text with each marked model reference (see MODEL_MARK) put as `name` says for its
/// instance and model, or as written when `name` gives None.
pub fn name_model_refs(text: &str, name: impl Fn(&str, &str) -> Option<String>) -> String {
    if !text.contains(MODEL_MARK) {
        return text.to_string();
    }
    let mut out = String::new();
    for (n, part) in text.split(MODEL_MARK).enumerate() {
        let reference = part.strip_prefix("model:").and_then(|r| r.split_once('/'));
        match reference.filter(|_| n % 2 == 1).and_then(|(instance, model)| name(instance, model)) {
            Some(named) => out.push_str(&named),
            None => out.push_str(part),
        }
    }
    out
}

/// A table row's cells: pipes split them unless escaped (`\|`); the outer ones are optional.
fn table_cells(line: &str) -> Vec<String> {
    let line = line.trim();
    let line = line.strip_prefix('|').unwrap_or(line);
    let line = if line.ends_with('|') && !line.ends_with("\\|") { &line[..line.len() - 1] } else { line };
    let mut cells = vec![String::new()];
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                chars.next();
                cells.last_mut().unwrap().push('|');
            }
            '|' => cells.push(String::new()),
            c => cells.last_mut().unwrap().push(c),
        }
    }
    cells.iter().map(|cell| cell_text(cell)).collect()
}

/// A cell as plain text, not game markup: the addon pads it, then escapes it.
fn cell_text(cell: &str) -> String {
    let p = patterns();
    inline(&p.br.replace_all(cell, " "), "$1").replace('\t', " ").trim().to_string()
}

/// The table that starts at `lines[at]`, if one does (a header row, then a delimiter row
/// with as many cells), and the line after it. Its body runs to the first line without a
/// pipe; a row's missing cells are empty and its extra ones dropped.
fn table_at(lines: &[&str], at: usize) -> Option<(Value, usize)> {
    let p = patterns();
    let (head, delimiter) = (lines[at], *lines.get(at + 1)?);
    if !head.contains('|') || !delimiter.contains('|') || !p.delimiter.is_match(delimiter) || p.fence.is_match(head) {
        return None;
    }
    let header = table_cells(head);
    let marks: Vec<&str> = delimiter.trim().trim_start_matches('|').trim_end_matches('|').split('|').map(str::trim).collect();
    if marks.len() != header.len() {
        return None;
    }
    let align: String = marks
        .iter()
        .map(|m| match (m.starts_with(':'), m.ends_with(':')) {
            (true, true) => 'c',
            (false, true) => 'r',
            _ => 'l',
        })
        .collect();
    let mut rows = vec![header];
    let mut end = at + 2;
    while end < lines.len() && lines[end].contains('|') && !p.fence.is_match(lines[end]) {
        let mut row = table_cells(lines[end]);
        row.resize(marks.len(), String::new());
        rows.push(row);
        end += 1;
    }
    let mut table = json!({ "t": rows });
    if align.contains(['c', 'r']) {
        table["a"] = json!(align);
    }
    Some((table, end))
}

/// A message cut into what the addon draws in its own font: prose (a string of WoW markup,
/// inline code between CODE_OPEN and CODE_CLOSE), code blocks ({ c = code }) and tables
/// ({ t = rows, the header first; a = each column's alignment, "l", "c" or "r", when one
/// isn't "l" }). Code and cells are plain text, not markup. Empty when the message has no
/// code and no table: its `wow_text` draws it the same.
pub fn wow_segments(markdown: &str, max_length: usize) -> Vec<Value> {
    let p = patterns();
    let mut text = markdown.replace("\r\n", "\n").replace('\r', "\n").replace([CODE_OPEN, CODE_CLOSE, LINK_MARK, MODEL_MARK], "");
    if text.chars().count() > max_length {
        text = text.chars().take(max_length).collect::<String>() + "\n…";
    }
    let lines: Vec<&str> = text.split('\n').collect();
    let mut segments = Vec::new();
    let mut prose: Vec<String> = Vec::new();
    let mut special = false;
    let flush = |prose: &mut Vec<String>, segments: &mut Vec<Value>| {
        let joined = p.gaps.replace_all(&prose.join("\n"), "\n\n").trim().to_string();
        if !joined.is_empty() {
            segments.push(json!(joined));
        }
        prose.clear();
    };
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if p.fence.is_match(line) {
            flush(&mut prose, &mut segments);
            special = true;
            // A fence indented in a list: its code loses that much indent.
            let indent = line.len() - line.trim_start_matches(' ').len();
            let mut code = Vec::new();
            i += 1;
            while i < lines.len() && !p.fence.is_match(lines[i]) {
                let l = lines[i];
                let spaces = l.len() - l.trim_start_matches(' ').len();
                code.push(l[spaces.min(indent)..].replace('\t', "    ").trim_end().to_string());
                i += 1;
            }
            i += 1;
            while code.last().is_some_and(String::is_empty) {
                code.pop();
            }
            if !code.is_empty() {
                segments.push(json!({ "c": code.join("\n") }));
            }
            continue;
        }
        if let Some((table, end)) = table_at(&lines, i) {
            flush(&mut prose, &mut segments);
            special = true;
            segments.push(table);
            i = end;
            continue;
        }
        let l = prose_line(&line.replace('|', "||"), &format!("{CODE_OPEN}$1{CODE_CLOSE}"));
        special |= l.contains(CODE_OPEN);
        prose.push(l);
        i += 1;
    }
    flush(&mut prose, &mut segments);
    if !special {
        return Vec::new();
    }
    // What the game's fonts can't draw, put as what they can: prose in the text font, code
    // and tables in JetBrains Mono.
    for segment in &mut segments {
        match segment {
            Value::String(prose) => *prose = prose_drawable(prose),
            Value::Object(fields) => {
                if let Some(Value::String(code)) = fields.get_mut("c") {
                    *code = drawable(code, Font::Mono);
                }
                for row in fields.get_mut("t").and_then(Value::as_array_mut).into_iter().flatten() {
                    for cell in row.as_array_mut().into_iter().flatten() {
                        if let Value::String(text) = cell {
                            *text = drawable(text, Font::Mono);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    segments
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
    fn code_and_tables_become_segments() {
        let cases: Value = serde_json::from_str(include_str!("../tests/fixtures/segments.json")).unwrap();
        for case in cases.as_array().unwrap() {
            let input = case["input"].as_str().unwrap();
            assert_eq!(json!(wow_segments(input, 6000)), case["segments"], "{input}");
        }
    }

    #[test]
    fn thread_links_are_marked_for_their_name_and_code_keeps_them() {
        let link = "t3-thread://v2/e1/thread:delegated-task:command%3Amcp%3Ax%3Adelegate-task%3Ar1";
        let marked = format!("{LINK_MARK}e1/thread:delegated-task:command%3Amcp%3Ax%3Adelegate-task%3Ar1{LINK_MARK}");
        assert_eq!(wow_text(&format!("Sol is reviewing in <{link}>. Then"), 6000), format!("Sol is reviewing in {marked}. Then"));
        assert_eq!(wow_text(&format!("see {link}."), 6000), format!("see {marked}."));
        assert_eq!(wow_text(&format!("[the review]({link})"), 6000), "the review");
        assert_eq!(wow_text(&format!("[{link}]({link})"), 6000), marked);
        assert_eq!(wow_text(&format!("`<{link}>`"), 6000), format!("{CODE}<{link}>|r"));
        assert_eq!(wow_text("a \u{4}x/y\u{4} b", 6000), "a x/y b");
        let segments = wow_segments(&format!("in <{link}> and `code`"), 6000);
        assert_eq!(segments, vec![json!(format!("in {marked} and {CODE_OPEN}code{CODE_CLOSE}"))]);
        let named = name_thread_links(&format!("in {marked}, then"), |env, thread| format!("[{env}|{thread}]"));
        assert_eq!(named, "in [e1|thread:delegated-task:command%3Amcp%3Ax%3Adelegate-task%3Ar1], then");
    }

    #[test]
    fn model_references_are_marked_as_t3_finds_them_and_code_keeps_them() {
        let m = |s: &str| format!("{MODEL_MARK}{s}{MODEL_MARK}");
        let opus = m("model:claudeAgent/claude-opus-5-5");
        assert_eq!(wow_text("ask model:claudeAgent/claude-opus-5-5 to review", 6000), format!("ask {opus} to review"));
        // Sentence punctuation and a bracket the slug doesn't open end it; its own stay.
        assert_eq!(wow_text("use model:claudeAgent/claude-opus-5-5.", 6000), format!("use {opus}."));
        assert_eq!(wow_text("(model:claudeAgent/claude-opus-5-5)", 6000), format!("({opus})"));
        assert_eq!(wow_text("model:x/GPT-6.1-Sol[1m], then", 6000), format!("{}, then", m("model:x/GPT-6.1-Sol[1m]")));
        assert_eq!(wow_text("\"model:opencode/anthropic/opus\"", 6000), format!("\"{}\"", m("model:opencode/anthropic/opus")));
        // Only after a space, comma, bracket or quote; an instance starts with a letter.
        assert_eq!(wow_text("mymodel:a/b and model:1a/b and model:a/", 6000), "mymodel:a/b and model:1a/b and model:a/");
        // Code keeps it as written.
        assert_eq!(wow_text("`model:a/b`", 6000), format!("{CODE}model:a/b|r"));
        assert_eq!(wow_segments("run `x` on model:a/b", 6000), vec![json!(format!("run {CODE_OPEN}x{CODE_CLOSE} on {}", m("model:a/b")))]);
        // T3's text can't hold the mark.
        assert_eq!(wow_text("a \u{5}b", 6000), "a b");

        let named = name_model_refs(&format!("ask {opus} or {}.", m("model:claudeAgent/gone")), |instance, model| {
            (instance == "claudeAgent" && model == "claude-opus-5-5").then(|| "Claude Opus 5.5".to_string())
        });
        assert_eq!(named, "ask Claude Opus 5.5 or model:claudeAgent/gone.", "one the machine lacks stays as written");
    }

    #[test]
    fn marks_the_fonts_lack_get_stand_ins_and_emoji_go() {
        let text = |s: &str| drawable(s, Font::Text);
        let mono = |s: &str| drawable(s, Font::Mono);
        assert_eq!(text("✅ tests pass, ❌ lint, ⚠️ flaky"), "✓ tests pass, ✗ lint, △ flaky");
        assert_eq!(text("a ➜ b ⟶ c, ⬅ d, ⬆ up ⬇ down, x ⟹ y"), "a → b → c, ← d, ↑ up ↓ down, x ⇒ y");
        assert_eq!(text("🟢 green 🔴 red ⚪ off, ★★☆ ➕ added ☐ todo ℹ️ note"), "● green ● red ○ off, ◆◆◇ + added □ todo i note");
        // What the fonts draw stays: arrows, check marks, punctuation, accents, CJK, a joiner in Devanagari.
        let kept = "→ ← ⇒ ✓ ✗ — … • “q” ≥ ș ț é 日本 क्\u{200D}ष";
        assert_eq!(text(kept), kept);
        // An emoji that starts a run takes the spaces after it; one that ends a sentence, the space before it.
        assert_eq!(text("🚀 Shipped\n🎉\tDone"), "Shipped\nDone");
        assert_eq!(text("Done 🎉 now. Done 🎉. Shipped 🚀!"), "Done now. Done. Shipped!");
        assert_eq!(text("“🚀 Launch” and \"Done 🎉\""), "“Launch” and \"Done\"");
        // An emoji alone on its line leaves no empty lines behind.
        assert_eq!(wow_text("🎉\n\nAll done!\n\n🚀\n\nNext\n\n👍", 6000), "All done!\n\nNext");
        assert_eq!(text("|cffffd100🚀 Launch 🎉|r"), "|cffffd100Launch|r");
        // A sequence goes whole: joined people and what they hold, skin tones, flags, tags.
        assert_eq!(text("by 👩🏽‍💻 🤷‍♀️ 👨‍⚕️ ✋🏽 at 🇺🇸 🏴\u{E0067}\u{E0062}\u{E0065}\u{E006E}\u{E0067}\u{E007F} end"), "by at end");
        // A stand-in after an emoji is still put; a keycap keeps its digit; ✉️ and the like go by their selector.
        assert_eq!(text("🚀🟢 live, step 1️⃣, ✉️ mail, ☎️ call"), "● live, step 1, mail, call");
        // Text left with nothing stays as written.
        assert_eq!(text("👍"), "👍");
        assert_eq!(text("plain ascii | stays"), "plain ascii | stays");
        // JetBrains Mono's own marks stay in code, and an emoji leaves a space so columns stay put.
        assert_eq!(mono("➜ ⚠ ⚡ ✕ ▸ ✅"), "➜ ⚠ ⚡ ✕ ▸ ✓");
        assert_eq!(mono("│ 🚀 Deploy │"), "│   Deploy │");
        assert_eq!(mono("│🚀🎉│ │👩‍💻│"), "│  │ │ │");
        assert_eq!(text("run \u{1}➜ 🚀 go\u{2} ➜"), "run \u{1}➜   go\u{2} →");
    }

    #[test]
    fn what_the_fonts_lack_is_put_after_the_markdown_is_read_and_copying_keeps_it() {
        // Markdown is read from T3's text: a dropped emoji or a stand-in can't make or break it.
        assert_eq!(wow_text("**✅ Done** ➜ next", 6000), "✓ Done → next");
        assert_eq!(wow_text("⭐ Recommended, **⭐⭐ great ⭐⭐**", 6000), "◆ Recommended, ◆◆ great ◆◆");
        assert_eq!(wow_text("⚠️[the docs](https://x)", 6000), "△the docs");
        // Nor make a pull request's number, which the game links: #️⃣157, ⚠️157.
        assert_eq!(wow_text("#️⃣ not a heading, #️⃣157 tests, ⚠️157 fails, ❗157 left", 6000), "not a heading, 157 tests, △157 fails, ‼157 left");
        // A heading of nothing but an emoji stays as written.
        assert_eq!(wow_text("## 🚀", 6000), "|cffffd100🚀|r");
        // A model reference is looked up as written.
        assert_eq!(wow_text("ask model:example/foo🚀 now ⏲️", 6000), "ask \u{5}model:example/foo🚀\u{5} now");
        // A joiner left dangling at the end of inline code doesn't take the code's end.
        let dangling = wow_segments("`go 🚀\u{200D}` later ➜ next", 6000);
        assert_eq!(dangling, vec![json!("\u{1}go  \u{2} later → next")]);
        assert_eq!(wow_text("🚀\u{200D}élan", 6000), "élan", "nor a letter after it");
        let table = wow_segments("| A | 🚀 |\n| --- | --- |\n| ✅ | x |", 6000);
        assert_eq!(table, vec![json!({ "t": [["A", "🚀"], ["✓", "x"]] })], "a cell left with nothing stays");
        let code = wow_segments("the `🚀` and `npm test`\n```\n❌ fail ➜ retry\n```", 6000);
        assert_eq!(code, vec![json!("the \u{1} \u{2} and \u{1}npm test\u{2}"), json!({ "c": "✗ fail ➜ retry" })]);
        assert_eq!(preview_text("⚠️ careful | 🚀 go", 200, 12), "⚠ careful ||   go");
        // A symbol the font has keeps its place, written as an emoji or not.
        assert_eq!(wow_text("↔️ ©️ ™️ ✓️", 6000), "↔ © ™ ✓");
        // The markup as T3 has it, for copying.
        assert_eq!(wow_markup("✅ `cat logs/✅.txt`", 6000), "✅ |cffc8c8a0cat logs/✅.txt|r");
        // One-line fields (titles, paths) stay as T3 has them.
        assert_eq!(plain("logs/✅.txt", 200), "logs/✅.txt");
    }

    #[test]
    fn plain_text_is_one_short_line() {
        assert_eq!(plain("  a |\n b\t c  ", 200), "a || b c");
        assert_eq!(plain("abcdef", 3), "abc");
    }
}
