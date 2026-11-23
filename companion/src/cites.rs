//! T3 Code's quotes of an assistant response ("Cite in composer", T3 0.1.11). A quote lives
//! in a message's own text as `[Assistant quote](t3-citation://v1/<env>/<thread>/<message>
//! ?text=…&start=…&end=…&prefix=…&suffix=…[&comment=…])`, read here as T3's
//! packages/shared/src/assistantCitations.ts reads it. A link T3 wouldn't take (an unknown
//! key, a value out of bounds) stays as written, as in T3.

use crate::lua::{drawable, Font};
use regex::Regex;
use serde_json::{json, Value};
use std::sync::OnceLock;

const MAX_TEXT: usize = 8_000;
const MAX_COMMENT: usize = 8_000;
const CONTEXT: usize = 32;
const MAX_ID: usize = 4_096;
const PREFIX: &str = "t3-citation://v1/";
// T3's own bound: percent encoding takes up to nine characters a UTF-16 unit.
const MAX_HREF: usize = 9 * (MAX_TEXT + MAX_COMMENT + 3 * MAX_ID + 2 * CONTEXT) + 1_000;
const MAX_SAFE: u64 = 9_007_199_254_740_991;

/// Where a quote of the message stands on its own line in the text, until it's drawn:
/// "\u{6}n\u{6}", n its place among the message's quotes. T3's text can't hold the mark: it's
/// taken out first.
pub const QUOTE_MARK: char = '\u{6}';

/// How much of a quote and its comment the game shows.
const SHOWN: usize = 1_000;

#[derive(Clone, Debug, PartialEq)]
pub struct Cite {
    pub env: String,
    pub thread: String,
    pub message: String,
    pub text: String,
    pub start: u64,
    pub end: u64,
    pub prefix: String,
    pub suffix: String,
    pub comment: Option<String>,
}

fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// Whitespace as JavaScript's `\s` and `trim` have it.
fn js_space(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
}

fn hex(b: u8) -> Option<u8> {
    (b as char).to_digit(16).map(|d| d as u8)
}

/// `decodeURIComponent`: None where it would throw (a bad escape, bytes that aren't UTF-8).
fn decode_component(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let (hi, lo) = (hex(*bytes.get(i + 1)?)?, hex(*bytes.get(i + 2)?)?);
            out.push(hi << 4 | lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// A query value as `URLSearchParams` decodes it: `+` a space, a bad escape kept as written,
/// bytes that aren't UTF-8 replaced.
fn decode_form(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() && hex(bytes[i + 1]).is_some() && hex(bytes[i + 2]).is_some() => {
                out.push(hex(bytes[i + 1]).unwrap() << 4 | hex(bytes[i + 2]).unwrap());
                i += 2;
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// An id as T3's schema takes it: trimmed, not blank, not too long.
fn id(raw: &str) -> Option<String> {
    let trimmed = decode_component(raw)?.trim_matches(js_space).to_string();
    (!trimmed.is_empty() && utf16_len(&trimmed) <= MAX_ID).then_some(trimmed)
}

/// A quote link's target, or None where T3 would leave the link as written.
pub fn parse_href(href: &str) -> Option<Cite> {
    // An empty fragment is no fragment, to a URL.
    let href = href.strip_suffix('#').unwrap_or(href);
    let rest = href.strip_prefix(PREFIX).filter(|_| href.len() <= MAX_HREF && !href.contains('#'))?;
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let parts: Vec<&str> = path.split('/').collect();
    // A URL takes "." and ".." out of its path, which leaves T3 the wrong number of parts.
    if parts.len() != 3 || parts.iter().any(|p| matches!(p.to_ascii_lowercase().as_str(), "." | ".." | "%2e" | "%2e%2e" | ".%2e" | "%2e.")) {
        return None;
    }
    let pairs: Vec<(String, String)> = query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (decode_form(k), decode_form(v))
        })
        .collect();
    let one = |key: &str| {
        let mut values = pairs.iter().filter(|(k, _)| k == key).map(|(_, v)| v.clone());
        match (values.next(), values.next()) {
            (Some(v), None) => Some(v),
            _ => None,
        }
    };
    let comment = one("comment");
    let multiple_comments = pairs.iter().filter(|(k, _)| k == "comment").count() > 1;
    if multiple_comments || pairs.len() != 5 + usize::from(comment.is_some()) {
        return None;
    }
    let number = |key: &str| {
        let v = one(key)?;
        (!v.is_empty() && v.len() <= 16 && v.bytes().all(|b| b.is_ascii_digit())).then(|| v.parse::<u64>().ok()).flatten().filter(|n| *n <= MAX_SAFE)
    };
    let cite = Cite {
        env: id(parts[0])?,
        thread: id(parts[1])?,
        message: id(parts[2])?,
        text: one("text")?,
        start: number("start")?,
        end: number("end")?,
        prefix: one("prefix")?,
        suffix: one("suffix")?,
        comment,
    };
    let fits = !cite.text.is_empty()
        && utf16_len(&cite.text) <= MAX_TEXT
        && cite.text.chars().any(|c| !js_space(c))
        && cite.comment.as_deref().is_none_or(|c| utf16_len(c) <= MAX_COMMENT)
        && utf16_len(&cite.prefix) <= CONTEXT
        && utf16_len(&cite.suffix) <= CONTEXT
        && cite.end > cite.start;
    fits.then_some(cite)
}

fn link() -> &'static Regex {
    static LINK: OnceLock<Regex> = OnceLock::new();
    // With the spaces around it, which its line of its own takes the place of.
    LINK.get_or_init(|| Regex::new(r"[ \t]*\[Assistant quote\]\((t3-citation://v1/[^\s)]+)\)[ \t]*").unwrap())
}

/// How a quote's link starts: a message of yours with one may need reading whole.
pub const LINK_START: &str = "[Assistant quote](t3-citation://v1/";

/// A link cut off at the end of a text that T3 cut short.
fn cut_link() -> &'static Regex {
    static CUT: OnceLock<Regex> = OnceLock::new();
    CUT.get_or_init(|| Regex::new(r"[ \t]*\[Assistant quote\]\(t3-citation://v1/[^\s)]*$").unwrap())
}

/// Whether a text T3 cut short may hold a quote's link: one, or the start of one at its end.
pub fn may_hold_link(text: &str) -> bool {
    text.contains(LINK_START) || (1..LINK_START.len()).any(|k| text.ends_with(&LINK_START[..k]))
}

/// The text cut to `max` characters (with "…" on a line of its own), never inside a quote's
/// mark, so the cut that `wow_text` and `wow_segments` make doesn't come into play.
pub fn capped(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(max.saturating_sub(2)).collect();
    if cut.matches(QUOTE_MARK).count() % 2 == 1 {
        let at = cut.rfind(QUOTE_MARK).unwrap_or(cut.len());
        cut.truncate(at);
    }
    cut + "\n…"
}

/// The text with each quote T3 would show on a line of its own as a mark (see QUOTE_MARK),
/// and those quotes in order. A link in code (a block, or between backticks) stays as
/// written, as code. In a text T3 cut short (`cut`), a link cut off at its end says so
/// instead of showing its encoded half.
pub fn take_cites(text: &str, cut: bool) -> (String, Vec<Cite>) {
    let text = text.replace(QUOTE_MARK, "");
    static FENCE: OnceLock<Regex> = OnceLock::new();
    let fence = FENCE.get_or_init(|| Regex::new(r"^\s*```").unwrap());
    let mut cites = Vec::new();
    let mut out = String::with_capacity(text.len());
    let (mut prose, mut in_fence) = (String::new(), false);
    static CODE: OnceLock<Regex> = OnceLock::new();
    let code = CODE.get_or_init(|| Regex::new(r"`[^`\n]+`").unwrap());
    let flush = |prose: &mut String, out: &mut String, cites: &mut Vec<Cite>| {
        let spans: Vec<(usize, usize)> = code.find_iter(prose).map(|m| (m.start(), m.end())).collect();
        let marked = link().replace_all(prose, |c: &regex::Captures| {
            let whole = c.get(0).unwrap();
            let in_code = spans.iter().any(|(s, e)| whole.start() < *e && *s < whole.end());
            match parse_href(&c[1]).filter(|_| !in_code) {
                Some(cite) => {
                    cites.push(cite);
                    format!("\n{QUOTE_MARK}{}{QUOTE_MARK}\n", cites.len() - 1)
                }
                None => c[0].to_string(),
            }
        });
        out.push_str(&marked);
        prose.clear();
    };
    for line in text.split_inclusive('\n') {
        if fence.is_match(line) {
            flush(&mut prose, &mut out, &mut cites);
            in_fence = !in_fence;
            out.push_str(line);
        } else if in_fence {
            out.push_str(line);
        } else {
            prose.push_str(line);
        }
    }
    flush(&mut prose, &mut out, &mut cites);
    let out = if cut && !in_fence { cut_link().replace(&out, "\n(quote cut off)").into_owned() } else { out };
    (out, cites)
}

/// Which quote a line is the mark of, if it is one.
pub fn mark_at(line: &str) -> Option<usize> {
    line.trim().strip_prefix(QUOTE_MARK)?.strip_suffix(QUOTE_MARK)?.parse().ok()
}

/// Text for the game to show: lines kept, other control characters out, at most SHOWN chars.
fn shown(text: &str) -> String {
    let text: String = text.replace("\r\n", "\n").chars().map(|c| if c == '\r' || c == '\t' { ' ' } else { c }).filter(|c| *c == '\n' || !c.is_control()).collect();
    let text = text.trim_matches(js_space);
    if text.chars().count() > SHOWN {
        text.chars().take(SHOWN).collect::<String>() + "…"
    } else {
        text.to_string()
    }
}

/// A quote as the segment the game draws: { q = quote, cm = comment }, both plain text in
/// what the game's fonts have. What finds its source (`_c`) goes before the view does (see
/// `link_quotes`).
pub fn segment(cite: &Cite) -> Value {
    let mut seg = json!({ "q": drawable(&shown(&cite.text), Font::Text) });
    if let Some(comment) = cite.comment.as_deref().map(shown).filter(|c| !c.is_empty()) {
        seg["cm"] = json!(drawable(&comment, Font::Text));
    }
    seg["_c"] = json!({
        "e": cite.env, "th": cite.thread, "m": cite.message, "text": cite.text,
        "start": cite.start, "end": cite.end, "prefix": cite.prefix, "suffix": cite.suffix,
    });
    seg
}

/// A segment as it goes to the game: without what finds a quote's source, for the view's size.
pub fn sent(seg: &Value) -> std::borrow::Cow<'_, Value> {
    match seg.get("_c") {
        Some(_) => {
            let mut seg = seg.clone();
            seg.as_object_mut().map(|m| m.remove("_c"));
            std::borrow::Cow::Owned(seg)
        }
        None => std::borrow::Cow::Borrowed(seg),
    }
}

/// A quote in game markup, for the message's text (what the copy box shows, and an older
/// addon draws): its lines after "> ", then the comment, in what the game's fonts have.
pub fn markup(cite: &Cite, dim: &str) -> String {
    let fit = |t: String| drawable(&t, Font::Text);
    let quote: Vec<String> = shown(&cite.text).split('\n').map(|l| format!("{dim}> {}|r", fit(l.replace('|', "||")))).collect();
    let mut out = quote.join("\n");
    if let Some(comment) = cite.comment.as_deref().map(shown).filter(|c| !c.is_empty()) {
        out.push_str(&format!("\n{}", fit(comment.replace('|', "||"))));
    }
    out
}

/// A short stable name for a quote, the same in every update: the game keeps it while it
/// loads earlier messages to find the quote's source.
fn quote_id(cite: &Cite) -> String {
    let mut hash: u32 = 0x811c_9dc5;
    for part in [&cite.env, &cite.thread, &cite.message, &cite.text, &cite.start.to_string(), &cite.end.to_string(), &cite.prefix, &cite.suffix] {
        for b in part.bytes().chain([0]) {
            hash = (hash ^ u32::from(b)).wrapping_mul(0x0100_0193);
        }
    }
    format!("{hash:08x}")
}

fn cite_of(c: &Value) -> Cite {
    let s = |k: &str| c[k].as_str().unwrap_or_default().to_string();
    Cite {
        env: s("e"),
        thread: s("th"),
        message: s("m"),
        text: s("text"),
        start: c["start"].as_u64().unwrap_or(0),
        end: c["end"].as_u64().unwrap_or(0),
        prefix: s("prefix"),
        suffix: s("suffix"),
        comment: None,
    }
}

/// Where a character sits: its piece, its first byte and the byte after it.
type Spot = (usize, usize, usize);

/// A piece of a message as the game draws it: game markup (prose), plain text (a code
/// block), or text that counts but can't be marked (a table).
pub enum Piece<'a> {
    Markup(&'a str),
    Raw(&'a str),
    Hidden(String),
}

/// The pieces the game draws an item from: its segments, else its text. Pieces are numbered
/// from 1 for segments, 0 for the text.
fn pieces(item: &Value) -> Vec<(usize, Piece<'_>)> {
    match item["g"].as_array().filter(|g| !g.is_empty()) {
        Some(segments) => segments
            .iter()
            .enumerate()
            .filter_map(|(n, seg)| {
                let piece = match seg {
                    Value::String(s) => Piece::Markup(s),
                    _ if seg["c"].is_string() => Piece::Raw(seg["c"].as_str().unwrap()),
                    _ if seg["t"].is_array() => {
                        let cells = seg["t"].as_array().unwrap().iter().flat_map(|row| row.as_array().into_iter().flatten()).filter_map(Value::as_str);
                        Piece::Hidden(cells.collect::<Vec<_>>().join(" "))
                    }
                    _ => return None,
                };
                Some((n + 1, piece))
            })
            .collect(),
        None => item["x"].as_str().map(|x| vec![(0, Piece::Markup(x))]).unwrap_or_default(),
    }
}

/// Each character a piece shows, with where it sits in the piece (bytes), as T3's page would
/// have it as text: no color codes or code marks, a doubled pipe one pipe, no list markers.
fn piece_chars(n: usize, piece: &Piece, out: &mut Vec<(char, Option<Spot>)>) {
    match piece {
        Piece::Hidden(text) => out.extend(text.chars().map(|c| (c, None))),
        Piece::Raw(text) => out.extend(text.char_indices().map(|(i, c)| (c, Some((n, i, i + c.len_utf8()))))),
        Piece::Markup(text) => {
            let b = text.as_bytes();
            let mut i = 0;
            let mut line_start = true;
            while i < b.len() {
                if line_start {
                    // A list's bullet or number, or a quote's ">", which T3 draws but doesn't
                    // hold as text. Spaces before it are whitespace either way.
                    let spaces = b[i..].iter().take_while(|c| **c == b' ').count();
                    let after = &text[i + spaces..];
                    let marker = if after.starts_with("• ") {
                        "• ".len()
                    } else if let Some(digits) = Some(after.bytes().take_while(u8::is_ascii_digit).count()).filter(|d| (1..=9).contains(d)) {
                        if after[digits..].starts_with(". ") || after[digits..].starts_with(") ") { digits + 2 } else { 0 }
                    } else if let Some(rest) = after.strip_prefix('>') {
                        1 + usize::from(rest.starts_with(' '))
                    } else {
                        0
                    };
                    if marker > 0 {
                        out.extend((0..spaces).map(|_| (' ', None)));
                        i += spaces + marker;
                    }
                    line_start = false;
                    continue;
                }
                match b[i] {
                    b'|' if b.get(i + 1) == Some(&b'|') => {
                        out.push(('|', Some((n, i, i + 2))));
                        i += 2;
                    }
                    b'|' if b.get(i + 1) == Some(&b'r') => i += 2,
                    b'|' if b.get(i + 1) == Some(&b'c') && b.len() >= i + 10 && b[i + 2..i + 10].iter().all(u8::is_ascii_hexdigit) => i += 10,
                    0x01..=0x06 => i += 1,
                    _ => {
                        let c = text[i..].chars().next().unwrap();
                        out.push((c, Some((n, i, i + c.len_utf8()))));
                        line_start = c == '\n';
                        i += c.len_utf8();
                    }
                }
            }
        }
    }
}

fn normalized_units(text: &str) -> Vec<u16> {
    let mut out = Vec::new();
    let mut space = false;
    for c in text.chars() {
        if js_space(c) {
            if !space {
                out.push(u16::from(b' '));
            }
            space = true;
        } else {
            space = false;
            let mut buf = [0u16; 2];
            out.extend_from_slice(c.encode_utf16(&mut buf));
        }
    }
    out
}

/// Where the quote is in `text` (UTF-16 units of whitespace-normalized text), as T3's
/// findAssistantCitationText finds it: the saved place if its context still matches, else
/// the one place whose context matches, else the quote's only place. Text that repeats
/// with no context to tell the places apart is found nowhere: a guess could mark the wrong
/// claim. In a source T3 cut short (`whole` false) the quote's only place in what was read
/// may not be its only place: it's found by its context alone.
fn find(text: &[u16], cite: &Cite, whole: bool) -> Option<(usize, usize)> {
    let quote = normalized_units(&cite.text);
    if quote.iter().all(|u| *u == u16::from(b' ')) {
        return None;
    }
    let prefix = normalized_units(&cite.prefix);
    let suffix = normalized_units(&cite.suffix);
    let context = |start: usize, end: usize| text[start.saturating_sub(prefix.len())..start] == prefix[..] && text.get(end..(end + suffix.len()).min(text.len())) == Some(&suffix[..]);
    let saved = match (usize::try_from(cite.start), usize::try_from(cite.end)) {
        (Ok(s), Ok(e)) => e.checked_sub(s) == Some(quote.len()) && text.get(s..e) == Some(&quote[..]) && context(s, e),
        _ => false,
    };
    let (s, e) = (cite.start as usize, cite.end as usize);
    let mut at = saved.then_some((s, e));
    let (mut only, mut count) = (None, 0);
    let mut start = 0;
    while start + quote.len() <= text.len() {
        if text[start..start + quote.len()] == quote[..] {
            let end = start + quote.len();
            count += 1;
            only = Some((start, end));
            if context(start, end) {
                if at.is_some_and(|(a, _)| a != start) {
                    return None;
                }
                at = Some((start, end));
            }
        }
        start += 1;
    }
    at.or(if count == 1 && whole { only } else { None })
}

/// Where a quote falls in an item's pieces: (piece, first byte, byte after), one a piece it
/// touches that the game can mark. None when the quote isn't found in it.
pub fn locate(item: &Value, cite: &Cite) -> Option<Vec<Spot>> {
    let whole = item["_t"] != true;
    let mut chars = Vec::new();
    for (k, (n, piece)) in pieces(item).iter().enumerate() {
        // Pieces are blocks: T3's text has a line between them.
        if k > 0 {
            chars.push(('\n', None));
        }
        piece_chars(*n, piece, &mut chars);
    }
    // Whitespace normalized, in UTF-16 units, each unit with where its character sits.
    let mut units: Vec<u16> = Vec::new();
    let mut origin: Vec<Option<Spot>> = Vec::new();
    let mut space = false;
    for (c, at) in chars {
        if js_space(c) {
            if !space {
                units.push(u16::from(b' '));
                origin.push(None);
            }
            space = true;
            continue;
        }
        space = false;
        let mut buf = [0u16; 2];
        for unit in c.encode_utf16(&mut buf) {
            units.push(*unit);
            origin.push(at);
        }
    }
    // The game draws marks its fonts lack as others (✅ as ✓) and leaves emoji out: the quote
    // is found as T3 has it, else as the game draws it.
    let fit = |t: &str| drawable(t, Font::Text);
    let drawn = Cite { text: fit(&cite.text), prefix: fit(&cite.prefix), suffix: fit(&cite.suffix), ..cite.clone() };
    let (start, end) = find(&units, cite, whole).or_else(|| if drawn == *cite { None } else { find(&units, &drawn, whole) })?;
    let mut ranges: Vec<Spot> = Vec::new();
    for (n, from, to) in origin[start..end].iter().flatten() {
        match ranges.last_mut() {
            Some(last) if last.0 == *n => last.2 = last.2.max(*to),
            _ => ranges.push((*n, *from, *to)),
        }
    }
    Some(ranges)
}

/// Ties each quote in the view to its source. A quote of a message the view shows gets `i`,
/// that item's place (from 1), and the item gets `hl` = { { quote id, piece, first byte,
/// last byte }, … } (from 1, as Lua counts) where the quoted text is. A quote of a message
/// in this thread that isn't loaded gets `ea` while earlier messages can be loaded, `far`
/// when the view already holds as much as the game is shown (`_cap`); one of another
/// thread its machine (`e`) and thread (`th`). With none of these its source is gone. Every
/// quote gets `id`. Done on the view as it goes to the game, after its text is final; takes
/// out what it was built with (`_m`, `_t`, `_c`, `_cap`).
pub fn link_quotes(view: &mut Value, env: &str, thread: &str, local_env: Option<&str>) {
    let more = view["more"] == true;
    let capped = view.as_object_mut().and_then(|v| v.remove("_cap")).is_some();
    let Some(items) = view["items"].as_array_mut() else { return };
    let ids: Vec<Option<String>> = items.iter().map(|it| it["_m"].as_str().filter(|_| it["r"] == "a").map(str::to_string)).collect();
    let here = |e: &str| e == env || ((env.is_empty() || Some(env) == local_env) && Some(e) == local_env);
    let mut marks: Vec<(usize, Value)> = Vec::new();
    for item in items.iter_mut() {
        let Some(segments) = item.get_mut("g").and_then(Value::as_array_mut) else { continue };
        for seg in segments.iter_mut().filter(|s| s["_c"].is_object()) {
            let cite = cite_of(&seg["_c"]);
            let id = quote_id(&cite);
            if !here(&cite.env) || cite.thread != thread {
                seg["e"] = json!(cite.env);
                seg["th"] = json!(cite.thread);
            } else if let Some(source) = ids.iter().position(|m| m.as_deref() == Some(cite.message.as_str())) {
                seg["i"] = json!(source + 1);
                // The same quote twice (again in a later message, say) is marked once.
                if !marks.iter().any(|(s, m)| *s == source && m["id"] == id.as_str()) {
                    marks.push((source, json!({ "id": id, "cite": seg["_c"].clone() })));
                }
            } else if capped {
                seg["far"] = json!(1);
            } else if more {
                seg["ea"] = json!(1);
            }
            seg["id"] = json!(id);
            if let Some(map) = seg.as_object_mut() {
                map.remove("_c");
            }
        }
    }
    for (source, mark) in marks {
        let cite = cite_of(&mark["cite"]);
        let Some(ranges) = locate(&items[source], &cite) else { continue };
        let found: Vec<Value> = ranges.into_iter().map(|(n, from, to)| json!([mark["id"], n, from + 1, to])).collect();
        match items[source].get_mut("hl").and_then(Value::as_array_mut) {
            Some(list) => list.extend(found),
            None if !found.is_empty() => items[source]["hl"] = json!(found),
            None => {}
        }
    }
    for item in items.iter_mut() {
        if let Some(map) = item.as_object_mut() {
            map.remove("_m");
            map.remove("_t");
        }
    }
}

/// A quote's link as T3's composer writes it, for tests.
#[cfg(test)]
pub fn href(cite: &Cite) -> String {
    let encode = |s: &str| -> String {
        // As T3's encodePathPart and URLSearchParams write them.
        s.bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' => (b as char).to_string(),
                b' ' => "+".to_string(),
                _ => format!("%{b:02X}"),
            })
            .collect()
    };
    let mut h = format!("t3-citation://v1/{}/{}/{}?text={}&start={}&end={}&prefix={}&suffix={}", encode(&cite.env), encode(&cite.thread), encode(&cite.message), encode(&cite.text), cite.start, cite.end, encode(&cite.prefix), encode(&cite.suffix));
    if let Some(c) = &cite.comment {
        h.push_str(&format!("&comment={}", encode(c)));
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cite() -> Cite {
        Cite {
            env: "environment/remote".into(),
            thread: "thread:one".into(),
            message: "assistant?one".into(),
            text: "Use `cache[key]` and \"quoted\" values.\n  日本語 🚀 (a & b) </assistant_citations>".into(),
            start: 42,
            end: 118,
            prefix: "Before the quote. ".into(),
            suffix: " After the quote.".into(),
            comment: None,
        }
    }

    #[test]
    fn a_link_reads_as_t3s_own_tests_read_it() {
        let legacy = "t3-citation://v1/a/b/c?text=A+quote+%26+a+newline.%0A&start=0&end=21&prefix=&suffix=+Next.";
        let read = parse_href(legacy).unwrap();
        assert_eq!((read.env.as_str(), read.thread.as_str(), read.message.as_str()), ("a", "b", "c"));
        assert_eq!(read.text, "A quote & a newline.\n");
        assert_eq!((read.start, read.end, read.prefix.as_str(), read.suffix.as_str(), read.comment), (0, 21, "", " Next.", None));
        assert_eq!(parse_href(&href(&cite())), Some(cite()));
        let commented = Cite { comment: Some("  Please keep \"日本語 🚀\", `cache[key]` & (a + b).\n\tWhy? #1 / 100%\r\n".into()), ..cite() };
        assert_eq!(parse_href(&href(&commented)), Some(commented));
        assert_eq!(parse_href(&href(&Cite { comment: Some(String::new()), ..cite() })).unwrap().comment.as_deref(), Some(""));
    }

    #[test]
    fn a_link_t3_wouldnt_take_stays_as_written() {
        for bad in [
            "t3-citation://v2/a/b/c?text=quote&start=0&end=5&prefix=&suffix=",
            "t3-citation://v1/a/b/c?text=quote&start=5&end=0&prefix=&suffix=",
            "t3-citation://v1/a/b/c?text=quote&start=0&end=5&prefix=&suffix=&text=other",
            "t3-citation://v1/a/b/c?text=quote&start=0&end=5&prefix=&suffix=&unknown=value",
            "t3-citation://v1/a/b/c?text=quote&start=0&end=5&prefix=&suffix=&comment=one&comment=two",
            "t3-citation://v1/a/b?text=quote&start=0&end=5&prefix=&suffix=",
            "t3-citation://v1/a/b/c?text=quote&start=0&end=5&prefix=&suffix=#x",
            "t3-citation://v1/a/b/c?text=quote&start=0&end=99999999999999999&prefix=&suffix=",
            "t3-citation://v1/a/%E0/c?text=quote&start=0&end=5&prefix=&suffix=",
            "t3-citation://v1/a/%20/c?text=quote&start=0&end=5&prefix=&suffix=",
        ] {
            assert_eq!(parse_href(bad), None, "{bad}");
            let text = format!("[Assistant quote]({bad})");
            assert_eq!(take_cites(&text, false), (text.clone(), vec![]));
        }
        assert_eq!(parse_href(&href(&Cite { text: "a".repeat(MAX_TEXT + 1), ..cite() })), None);
        assert_eq!(parse_href(&href(&Cite { prefix: "a".repeat(33), ..cite() })), None);
        assert_eq!(parse_href(&href(&Cite { text: "  ".into(), ..cite() })), None);
        // A link labelled otherwise, or a bare one, is ordinary text.
        let h = href(&cite());
        let text = format!("{h} [Other quote]({h})");
        assert_eq!(take_cites(&text, false).1, vec![]);
    }

    #[test]
    fn each_quote_takes_a_line_of_its_own() {
        let h = href(&cite());
        let (text, cites) = take_cites(&format!("About [Assistant quote]({h}), explain\u{6} this.[Assistant quote]({h})"), false);
        assert_eq!(text, "About\n\u{6}0\u{6}\n, explain this.\n\u{6}1\u{6}\n");
        assert_eq!(cites, vec![cite(), cite()]);
        assert_eq!(mark_at("\u{6}1\u{6}"), Some(1));
        assert_eq!(mark_at("x\u{6}1\u{6}"), None);
    }

    #[test]
    fn a_link_in_code_stays_code_and_a_cut_link_says_so() {
        let h = href(&cite());
        let (text, cites) = take_cites(&format!("```\n[Assistant quote]({h})\n```\n`[Assistant quote]({h})` [Assistant quote]({h}#)"), false);
        assert_eq!(text, format!("```\n[Assistant quote]({h})\n```\n`[Assistant quote]({h})`\n\u{6}0\u{6}\n"));
        assert_eq!(cites.len(), 1, "an empty fragment is no fragment");
        let half = format!("Look: [Assistant quote]({}", &h[..h.len() / 2]);
        assert_eq!(take_cites(&half, true), ("Look:\n(quote cut off)".to_string(), vec![]));
        assert_eq!(take_cites(&half, false), (half.clone(), vec![]), "a whole text keeps what it says");
        assert!(may_hold_link("text [Assistant qu") && may_hold_link(&half) && !may_hold_link("text"));
        // Cut to a size, never inside a quote's mark.
        assert_eq!(capped("ab\n\u{6}12\u{6}\ncd", 7), "ab\n\n…");
        assert_eq!(capped("short", 7), "short");
    }

    fn at(text: &str, quote: &str, start: u64, prefix: &str, suffix: &str) -> Option<(usize, usize)> {
        let cite = Cite { text: quote.into(), start, end: start + utf16_len(quote) as u64, prefix: prefix.into(), suffix: suffix.into(), ..cite() };
        find(&normalized_units(text), &cite, true)
    }

    #[test]
    fn a_quote_is_found_as_t3_finds_it() {
        // At its saved place, its context matching.
        assert_eq!(at("one two three", "two", 4, "one ", " three"), Some((4, 7)));
        // Moved: found by its context.
        assert_eq!(at("zero one two three", "two", 4, "one ", " three"), Some((9, 12)));
        // Repeated, told apart by context.
        assert_eq!(at("a x b. c x d.", "x", 0, "c ", " d"), Some((9, 10)));
        // Repeated, with context that matches neither: nowhere, not a guess.
        assert_eq!(at("a x b. c x d.", "x", 2, "q ", " r"), None);
        // Repeated with the same context, though one sits at the saved place: nowhere.
        assert_eq!(at("a x a x", "x", 2, "a ", ""), None);
        // Once in the text, its context changed: found anyway.
        assert_eq!(at("new words here", "words", 0, "old ", ""), Some((4, 9)));
        // Gone.
        assert_eq!(at("nothing", "words", 0, "", ""), None);
        // In a source T3 cut short, its only place in what was read isn't enough.
        let words = Cite { text: "words".into(), start: 0, end: 5, prefix: "old ".into(), suffix: String::new(), ..cite() };
        assert_eq!(find(&normalized_units("new words here"), &words, false), None);
        // Whitespace counts as one space; offsets are UTF-16 units.
        assert_eq!(at("🚀 a\n\n  b", "a b", 3, "🚀 ", ""), Some((3, 6)));
    }

    fn source(x: &str) -> Value {
        json!({ "r": "a", "x": x, "_m": "m1" })
    }

    fn quote_of(text: &str) -> Cite {
        Cite { env: "local".into(), thread: "t1".into(), message: "m1".into(), text: text.into(), start: 0, end: utf16_len(text) as u64, prefix: String::new(), suffix: String::new(), comment: None }
    }

    #[test]
    fn a_quote_is_found_in_the_game_text_its_source_is_drawn_from() {
        // Color codes, doubled pipes and list bullets aren't in T3's text; the range covers
        // the bytes the game draws.
        let item = source("|cffffd100Heading|r\n• one || two\n• three");
        let ranges = locate(&item, &quote_of("Heading one | two three")).unwrap();
        let x = item["x"].as_str().unwrap();
        assert_eq!(ranges.len(), 1);
        let (n, from, to) = ranges[0];
        assert_eq!((n, &x[from..to]), (0, "Heading|r\n• one || two\n• three"));
        // In segments: per piece, a table counting but not marked.
        let item = json!({ "r": "a", "x": "", "g": ["Run \u{1}npm test\u{2} now", { "c": "ok | fail" }, { "t": [["k", "v"]] }, "after"], "_m": "m1" });
        let ranges = locate(&item, &quote_of("npm test now ok | fail k v after")).unwrap();
        assert_eq!(ranges, vec![(1, 5, 18), (2, 0, 9), (4, 0, 5)]);
        // Marks the game draws as others, and emoji it leaves out.
        let item = source(&drawable("Status:\n✅ Tests pass on Windows 🚀 today", Font::Text));
        let x = item["x"].as_str().unwrap().to_string();
        let (n, from, to) = locate(&item, &quote_of("✅ Tests pass on Windows 🚀 today")).unwrap()[0];
        assert_eq!((n, &x[from..to]), (0, "✓ Tests pass on Windows today"));
    }

    #[test]
    fn quotes_are_tied_to_their_sources_as_the_view_goes_out() {
        let mut view = json!({ "more": true, "items": [
            source("Alpha beta gamma."),
            { "r": "u", "x": "", "g": [
                segment(&quote_of("beta")),
                segment(&Cite { message: "older".into(), ..quote_of("x") }),
                segment(&Cite { thread: "t2".into(), ..quote_of("y") }),
                segment(&quote_of("changed since")),
            ] },
        ] });
        link_quotes(&mut view, "", "t1", Some("local"));
        let g = &view["items"][1]["g"];
        assert_eq!(g[0]["i"], 1);
        assert_eq!(g[1]["ea"], 1);
        assert_eq!((g[2]["e"].as_str(), g[2]["th"].as_str()), (Some("local"), Some("t2")));
        assert_eq!(g[3]["i"], 1, "a quote whose text changed still goes to its message");
        let id = g[0]["id"].as_str().unwrap();
        assert_eq!(view["items"][0]["hl"], json!([[id, 0, 7, 10]]), "only the quote that's still there is marked");
        assert!(!view.to_string().contains("\"_"), "{view}");
        // The same quote twice is marked once; a source as far back as the game is shown is far.
        let mut view = json!({ "more": true, "_cap": true, "items": [
            source("Alpha beta gamma."),
            { "r": "u", "x": "", "g": [segment(&quote_of("beta")), segment(&quote_of("beta")), segment(&Cite { message: "older".into(), ..quote_of("x") })] },
        ] });
        link_quotes(&mut view, "local", "t1", Some("local"));
        assert_eq!(view["items"][0]["hl"].as_array().unwrap().len(), 1, "{view}");
        assert_eq!(view["items"][1]["g"][2]["far"], 1);
        assert!(view["items"][1]["g"][2]["ea"].is_null() && view["_cap"].is_null());
        // With every message loaded, a missing source is gone.
        let mut view = json!({ "items": [{ "r": "u", "x": "", "g": [segment(&Cite { message: "older".into(), ..quote_of("x") })] }] });
        link_quotes(&mut view, "local", "t1", Some("local"));
        let seg = &view["items"][0]["g"][0];
        assert!(seg["i"].is_null() && seg["ea"].is_null() && seg["th"].is_null() && seg["id"].is_string(), "{seg}");
    }
}
