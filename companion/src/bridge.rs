//! The bridge between the game and T3: runs the game's commands against T3, reads T3's
//! threads, and publishes what the game shows into slot files, raising the signal files
//! that tell the game to read them.

use crate::clip::{self, Clipped};
use crate::codec::decode_cells;
use crate::config::Trust;
use crate::made_for::{self, older_t3, OlderT3};
use crate::lua::{name_model_refs, name_thread_links, plain, to_lua, wow_segments, wow_text, MODEL_MARK};
use crate::render::{page_height, tga, Bitmap, Painter, DRAWING_VERSION};
use crate::slots::{SlotWriter, IMAGE_COUNT, SIGNAL_COUNT, SLOT_COUNT};
use crate::snooze::{civil_from_days, days_from_civil};
use crate::t3::{T3Error, T3};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;

const WORKING: [&str; 4] = ["preparing", "queued", "starting", "running"];
const MIN_PUBLISH_GAP_MS: i64 = 1500;
/// Routine changes while the window is open: one update at most this often (200 slots
/// then last about 35 minutes of watching a busy thread, not 15).
const ROUTINE_GAP_MS: i64 = 10_000;
/// The parts of an update the game keeps from the one before while they stay the same:
/// the open thread's transcript runs to about 50 KB, the thread list to 25 KB, the model
/// picks to 20 KB. An update leaves out the ones the game holds (`Bridge::held`) and
/// names them in `same`; `hx` hashes every part, so the game can check it holds that one.
/// Keep in step with addon/T3WoW/Link.lua.
const KEPT_PARTS: [&str; 5] = ["view", "threads", "projects", "models", "picks"];
/// Slots up to this one carry every part: after a Reload UI the game starts again from
/// slot 1, holding nothing.
const WHOLE_SLOTS: u32 = 3;
/// While waiting, the game moves to the next signal file every SIGNAL_PERIOD seconds.
/// Same clock, same arithmetic: keep in step with addon/T3WoW/Link.lua.
pub const SIGNAL_PERIOD: i64 = 25;
const VIEW_MESSAGES: usize = 24;
const VIEW_CHARS: usize = 40_000;
/// The open thread is read in full at least this often; in between, only what's new.
const VIEW_FULL_READ_MS: i64 = 300_000;
/// An open thread that isn't in the lists (no updatedAt to compare) is read again this often.
const UNLISTED_VIEW_TTL_MS: i64 = 20_000;

/// Why the game was told to stop watching signals.
const SIGNALS_OUT: &str = "this WoW session used all its signal files; restart WoW for updates within a second";
/// Why, when the signal files are gone from the addon folder (something replaced or
/// cleaned it): the game can't be told an update is ready until they're back.
#[cfg(not(target_os = "macos"))]
const SIGNALS_LOST: &str = "the addon folder lost its signal files (something replaced or cleaned it): restart WoW once they're back (T3 WoW puts them back by itself, or choose Repair addon files in its tray menu)";
// A Mac: the menu bar app's item, or under `t3wow run` (no menu) `t3wow install`.
#[cfg(target_os = "macos")]
const SIGNALS_LOST: (&str, &str) = (
    "the addon folder lost its signal files (something replaced or cleaned it): restart WoW once they're back (T3 WoW puts them back by itself, or choose Repair addon files from its menu bar icon)",
    "the addon folder lost its signal files (something replaced or cleaned it): restart WoW once they're back (T3 WoW puts them back by itself, or run t3wow install)",
);
fn signals_lost() -> &'static str {
    crate::app::shown(SIGNALS_LOST)
}
/// Where "Commands from the game" is set: the tray menu, the Mac's menu bar, or under
/// `t3wow run` on a Mac (no menu) the config file.
#[cfg(not(target_os = "macos"))]
const TRUST_SETTING: &str = "the T3 WoW tray setting \"Commands from the game\"";
#[cfg(target_os = "macos")]
const TRUST_SETTING: (&str, &str) = (
    "the T3 WoW menu bar setting \"Commands from the game\"",
    "T3 WoW's setting \"Commands from the game\" (gameCommands in its config.json)",
);
/// The name the game gives this machine when the system doesn't say.
#[cfg(not(target_os = "macos"))]
const THIS_MACHINE: &str = "This PC";
#[cfg(target_os = "macos")]
const THIS_MACHINE: &str = "This Mac";
/// Why a page T3 drew can't be shown when its file is on the network or another machine.
#[cfg(not(target_os = "macos"))]
const FILE_NOT_HERE: &str = "its file isn't on this PC";
#[cfg(target_os = "macos")]
const FILE_NOT_HERE: &str = "its file isn't on this Mac";

/// Failed reads in a row before an active thread off the lists stops being watched.
const UNLISTED_MISSES: u32 = 5;

/// How long a working thread's pending-question count is trusted without a re-read.
const PENDING_RECHECK_MS: i64 = 20_000;
/// How long whether a thread watches its pull request is trusted without a re-read (a
/// watch can end, the PR merged, without the thread changing).
const WATCH_RECHECK_MS: i64 = 60_000;

fn working(status: &str) -> bool {
    WORKING.contains(&status)
}

/// The status a thread gets while its agent watches a pull request or waits on a subagent
/// after its run ended: T3's sidebar files it under Working as "Waiting", though its own
/// status says it ended.
const WATCHING: &str = "watching";

/// Working, or watching a pull request: either way the thread isn't done yet.
fn active(status: &str) -> bool {
    working(status) || status == WATCHING
}

/// A thread that may be watching a pull request: open on the sidebar, its run ended
/// without waiting on you, with a pull request not yet merged or closed. Only such threads
/// are asked whether they watch one.
fn may_watch(thread: &Value) -> bool {
    run_over(thread) && thread["pullRequests"].as_array().is_some_and(|prs| prs.iter().any(pull_request_open))
}

/// Open on the sidebar, with its run ended and not waiting on you.
fn run_over(thread: &Value) -> bool {
    let status = str_of(&thread["status"]);
    thread["settled"] != true && !working(status) && !needs_you(status)
}

/// The threads with a subagent in a list of threads still going. Forks have a parent too,
/// but it doesn't wait on them.
fn waiting_parents(list: &Value) -> HashSet<String> {
    let threads = list["threads"].as_array().into_iter().flatten();
    threads.filter(|t| t["relationshipToParent"] == "subagent").filter_map(|t| t["parentThreadId"].as_str().map(str::to_string)).collect()
}

fn pull_request_open(pr: &Value) -> bool {
    !matches!(str_of(&pr["state"]), "merged" | "closed")
}

/// The pull requests linked to a thread that have a number to show.
fn numbered_pull_requests(thread: &Value) -> Vec<&Value> {
    thread["pullRequests"].as_array().map_or_else(Vec::new, |prs| prs.iter().filter(|pr| pr["number"].is_u64()).collect())
}

/// GitLab numbers its merge requests with "!", on any host.
fn pull_request_number(pr: &Value) -> String {
    let sign = if str_of(&pr["url"]).contains("/merge_requests/") { '!' } else { '#' };
    format!("{sign}{}", pr["number"])
}

/// A thread's linked pull requests in a few characters, for its row: the first still open
/// (in a stack, the next to merge), else the last linked, and how many more there are.
/// With the state of the one named, which colors it.
fn pull_request_label(thread: &Value) -> Option<(String, String)> {
    let prs = numbered_pull_requests(thread);
    let shown = prs.iter().find(|pr| pull_request_open(pr)).or(prs.last())?;
    let number = pull_request_number(shown);
    let label = if prs.len() > 1 { format!("{number} +{}", prs.len() - 1) } else { number };
    Some((label, str_of(&shown["state"]).to_string()))
}

/// Whether a thread's row shows its age from its latest run (it has one, it's at rest and
/// not settled), which only a thread read has.
fn ages_from_run(thread: &Value) -> bool {
    let at_rest = matches!(str_of(&thread["status"]), "completed" | "idle" | "interrupted" | "cancelled" | "rolled_back");
    at_rest && thread["settled"] != true && !str_of(&thread["latestRunId"]).is_empty()
}

/// A run's newest time (requested, started or completed), in epoch seconds; 0 if none.
fn run_activity(run: &Value) -> i64 {
    ["requestedAt", "startedAt", "completedAt"].iter().map(|k| iso_seconds(str_of(&run[*k]))).max().unwrap_or(0)
}

/// Only a web page goes to the browser: T3's record could hold anything Windows would run.
fn is_web_url(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://")
}

/// Every pull request linked to a thread, for its row and the open thread's header.
fn pull_request_links(thread: &Value) -> Vec<Value> {
    numbered_pull_requests(thread)
        .into_iter()
        .map(|pr| {
            let mut link = json!({ "n": pull_request_number(pr), "s": str_of(&pr["state"]) });
            if is_web_url(str_of(&pr["url"])) {
                link["u"] = pr["url"].clone();
            }
            link
        })
        .collect()
}

/// Waiting on a question, an approval, or a card only T3 can answer.
fn needs_you(status: &str) -> bool {
    status == "waiting" || status == "approval" || status == CARD
}

/// The status a working thread gets while its agent waits on a card that only T3's own
/// window can answer: a secret (`request_secret`) or a machine link. T3's sidebar says
/// "Awaiting Input" and notifies, but its thread list and pending count leave the card out:
/// only the card's item in the thread's activity says it waits.
const CARD: &str = "card";

/// How many of a working thread's latest activity items its pending check reads, to find a
/// waiting card: the agent stops at the card, so it is among the last few.
const CARD_LOOKBACK: u64 = 4;

/// What a waiting card asks for, from one of a working thread's latest activity items: its
/// kind ("secret" or "link") and T3's label for it. A card inherited from another thread
/// (a fork) can only be answered there, and T3 doesn't count it.
fn waiting_card(item: &Value) -> Option<Card> {
    let kind = match str_of(&item["type"]) {
        "secret_request" => "secret",
        "link_request" => "link",
        _ => return None,
    };
    if item["status"] != "waiting" || item["visibility"].as_str().is_some_and(|v| v != "local") {
        return None;
    }
    Some(Card { kind, label: plain(str_of(&item["title"]), 120) })
}

/// The read that checks what a working thread waits on: its pending count, and its latest
/// few activity items (titles only), where a waiting card shows. `item_count` is the
/// thread's from its list.
fn pending_read_args(id: &str, item_count: u64) -> Value {
    let mut args = json!({ "threadId": id, "view": "activity", "limit": CARD_LOOKBACK, "maxCharsPerItem": 1 });
    if let Some(after) = item_count.checked_sub(CARD_LOOKBACK + 1) {
        args["afterPosition"] = json!(after);
    }
    args
}

/// T3 stores a run whose turn just ended as "waiting" until it has wrapped the run up, and
/// its own sidebar counts that as working: only a pending request waits on you. So what T3
/// says goes in as working, and "waiting" here always means its pending count said so.
fn read_as_working_if_waiting(thread: &mut Value) {
    if thread["status"] == "waiting" {
        thread["status"] = json!("running");
    }
}

/// T3's sidebar files subagent threads, and threads an agent launched, under the thread
/// they came from. Linked machines don't always apply includeSubagents, so check here too.
/// Subagents never show; threads an agent launched show under the thread that launched
/// them, as in T3's sidebar, and don't count in their group.
fn is_subagent(thread: &Value) -> bool {
    thread["parentThreadId"].is_string()
}

fn on_sidebar(thread: &Value) -> bool {
    !is_subagent(thread) && thread["createdBy"] != "agent"
}

/// How much of an agent-made thread's first message is read, to know where it came from.
const OPENING_CHARS: u64 = 120;

/// Whether a first message opens the way T3 opens a task delegated with a role.
fn opens_delegated_task(message: &str) -> bool {
    let line = message.lines().next().unwrap_or("");
    line.starts_with("Act as the ") && line.ends_with(" sub-agent for this task.")
}

/// Where an agent-made thread came from, as T3's read of it tells.
#[derive(Clone)]
struct Origin {
    /// The thread that launched it, when its first message names a sender.
    launcher: Option<String>,
    /// With no sender, whether T3 lists it (`find_launcher`).
    listed: bool,
    /// Whether a later read could tell otherwise: not for a sender, a role line or a
    /// listed thread.
    open: bool,
    /// The thread's `updatedAt` when this was read.
    updated: String,
}

/// Whether an activity item (its text cut short) is a page an agent showed with html_render.
fn is_page_item(text: &str) -> bool {
    static PAGE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    PAGE.get_or_init(|| regex::Regex::new(r#"^\s*\{\s*"toolName"\s*:\s*"[^"]*html_render""#).unwrap()).is_match(text)
}

/// The page's title, when its item's text (cut short) has it.
fn page_title(text: &str) -> String {
    static TITLE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let title = TITLE.get_or_init(|| regex::Regex::new(r#""title"\s*:\s*("(?:[^"\\]|\\.)*")"#).unwrap());
    title.captures(text).and_then(|c| serde_json::from_str::<String>(&c[1]).ok()).unwrap_or_default()
}

/// The largest page file read (T3 reads at most 25 MiB), and the largest snapshot of one
/// (T3 puts the page's local images in it).
const PAGE_FILE_BYTES: u64 = 25 * 1024 * 1024;
const PAGE_SNAPSHOT_BYTES: u64 = 64 * 1024 * 1024;

/// The HTML an html_render call (its item's text, as JSON) showed. A page passed inline is
/// in the call. One passed as a file (`path`) is drawn from the snapshot T3 saved when it
/// showed it, which is what T3 shows even after the file changes or goes; failing that, from
/// the file. Both are on the machine the thread runs on: `attachments` is T3's folder of
/// them when that's this PC, else None.
fn page_html(text: &str, attachments: Option<&std::path::Path>) -> Result<String, String> {
    let call: Value = serde_json::from_str(text).map_err(|_| "the page's item couldn't be read".to_string())?;
    let input = &call["input"];
    if let Some(html) = input["html"].as_str().filter(|h| !h.trim().is_empty()) {
        return Ok(html.to_string());
    }
    let Some(file) = input["path"].as_str().filter(|p| !p.trim().is_empty()) else { return Err("the page has no HTML".into()) };
    let attachments = attachments.ok_or("the page is a file on another machine")?;
    // The result's text when its structured form is missing (some providers drop it).
    let content: Value = call["output"]["content"].as_str().and_then(|c| serde_json::from_str(c).ok()).unwrap_or(Value::Null);
    let saved = [&call["output"]["structuredContent"], &content]
        .into_iter()
        .filter_map(|out| out["htmlRender"]["attachmentId"].as_str())
        .find(|id| !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        .and_then(|id| read_limited(&attachments.join(format!("{id}.html")), PAGE_SNAPSHOT_BYTES).ok());
    if let Some(snapshot) = saved {
        return Ok(without_t3_bootstrap(&snapshot));
    }
    // As T3 reads it: a full path on this PC (not the network), to an .html file.
    let network = file.starts_with("\\\\") || file.starts_with("//");
    let file = std::path::Path::new(file);
    if network || !file.is_absolute() {
        return Err(FILE_NOT_HERE.into());
    }
    let html_named = |p: &std::path::Path| p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("html") || e.eq_ignore_ascii_case("htm"));
    // The real file, so a link named .html can't reach another one.
    let real = std::fs::canonicalize(file).map_err(|_| "its file is gone".to_string())?;
    if !html_named(&real) {
        return Err("its file isn't an .html file".into());
    }
    read_limited(&real, PAGE_FILE_BYTES)
}

/// A text file of at most `limit` bytes.
fn read_limited(path: &std::path::Path, limit: u64) -> Result<String, String> {
    let meta = std::fs::metadata(path).ok().filter(|m| m.is_file()).ok_or("its file is gone")?;
    if meta.len() > limit {
        return Err("its file is too large".into());
    }
    std::fs::read_to_string(path).map_err(|_| "its file couldn't be read".to_string())
}

/// A page snapshot as the agent wrote it: without the theme and script T3 put at its top
/// (a light theme too, which the browser drawing it would pick), so it's drawn on T3's dark
/// theme like a page passed inline.
fn without_t3_bootstrap(html: &str) -> String {
    const THEME: &str = "<style id=\"t3-theme\">";
    let Some(start) = html.find(THEME) else { return html.to_string() };
    let Some(theme_len) = html[start..].find("</style>").map(|n| n + "</style>".len()) else { return html.to_string() };
    let mut end = start + theme_len;
    if html[end..].starts_with("<script>") {
        if let Some(n) = html[end..].find("</script>") {
            end += n + "</script>".len();
        }
    }
    format!("{}{}", &html[..start], &html[end..])
}

/// The images a message embeds from files on this PC (`![alt](C:\path)` or a file:// URL),
/// with their alt text.
fn embedded_files(markdown: &str) -> Vec<(String, PathBuf)> {
    static IMAGE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let image = IMAGE.get_or_init(|| regex::Regex::new(r#"!\[([^\]]*)\]\(\s*<?([^)<>"]+?)>?(?:\s+"[^"]*")?\s*\)"#).unwrap());
    image
        .captures_iter(markdown)
        .filter_map(|c| {
            let target = c[2].trim();
            let path = match target.strip_prefix("file:///").or_else(|| target.strip_prefix("file://")) {
                Some(rest) => crate::t3::percent_decode(rest),
                None => target.to_string(),
            };
            let bytes = path.as_bytes();
            let absolute = bytes.len() > 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && (bytes[2] == b'\\' || bytes[2] == b'/');
            // Off Windows a path starts at /, which file:/// has just taken off.
            #[cfg(not(windows))]
            let path = if !absolute && target.starts_with("file:///") { format!("/{path}") } else { path };
            #[cfg(not(windows))]
            let absolute = absolute || path.starts_with('/');
            absolute.then(|| (c[1].trim().to_string(), PathBuf::from(path)))
        })
        .collect()
}

/// What an embedded image file is as it is now (a changed file is drawn again); None when
/// it isn't a file on this PC.
fn file_key(path: &std::path::Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok().filter(|m| m.is_file())?;
    let modified = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
    Some(format!("file:{}:{}:{modified}:{DRAWING_VERSION}", path.display(), meta.len()))
}

fn hash_of(value: &Value) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.to_string().hash(&mut hasher);
    hasher.finish()
}

/// Which of a model's options is its effort (T3 has "effort" and "reasoningEffort").
fn is_effort(id: &str) -> bool {
    id.to_ascii_lowercase().contains("effort")
}

/// A machine's providers as the game lists them: the ones it can run, with their models and
/// each model's efforts. Disabled providers still come with models; T3 says so in `constraints`.
fn compact_providers(capabilities: &Value) -> Vec<Value> {
    let mut providers = Vec::new();
    for provider in capabilities["providers"].as_array().into_iter().flatten() {
        if provider["constraints"].as_array().is_some_and(|c| !c.is_empty()) {
            continue;
        }
        let mut models = Vec::new();
        for model in provider["models"].as_array().into_iter().flatten() {
            let id = str_of(&model["id"]);
            if id.is_empty() {
                continue;
            }
            let mut entry = json!({ "id": id, "t": plain(model_text(id, model["label"].as_str()), 40) });
            if let Some(effort) = model["options"].as_array().into_iter().flatten().find(|o| is_effort(str_of(&o["id"]))) {
                entry["o"] = effort["id"].clone();
                entry["ef"] = effort["values"].clone();
            }
            models.push(entry);
        }
        if models.is_empty() {
            continue;
        }
        let id = str_of(&provider["providerInstanceId"]);
        let name = provider["displayName"].as_str().unwrap_or(id);
        let mut entry = json!({ "id": id, "t": plain(name, 40), "m": models });
        // The game's @ finds models by their driver's name too, as T3 does; said only when
        // it isn't the instance's id.
        let driver = str_of(&provider["driverKind"]);
        if !driver.is_empty() && driver != id {
            entry["dk"] = json!(driver);
        }
        // Left out when the provider takes every mode.
        if provider["runtimeModes"].is_array() {
            entry["rm"] = provider["runtimeModes"].clone();
        }
        providers.push(entry);
    }
    providers
}

/// The most a provider lists in the game's picker.
const PICKS_KEPT: usize = 300;

/// What a provider's composer lists, from T3's cache of it, as the game's picker shows it:
/// `n` the name, `d` a short description, `c` when `/name` runs it, `s` when `$name` does. A
/// name T3 couldn't parse back out of a message is left out.
fn compact_picks(cache: &Value) -> Vec<Value> {
    let typable = |name: &str| !name.is_empty() && name.len() <= 64 && name.chars().all(|c| c.is_ascii_alphanumeric() || ":_-".contains(c));
    let listed = |key: &str| {
        let items = cache[key].as_array().map_or(&[][..], Vec::as_slice);
        items.iter().filter(|item| typable(str_of(&item["name"])) && item["enabled"] != false).cloned().collect::<Vec<Value>>()
    };
    let (commands, skills) = (listed("slashCommands"), listed("skills"));
    let is_in = |items: &[Value], name: &str| items.iter().any(|item| str_of(&item["name"]) == name);
    let mut picks: Vec<Value> = Vec::new();
    for item in commands.iter().chain(&skills).take(PICKS_KEPT) {
        let name = str_of(&item["name"]);
        if picks.iter().any(|pick| pick["n"] == name) {
            continue;
        }
        let mut pick = json!({ "n": name, "d": plain(str_of(&item["description"]), 60) });
        if is_in(&commands, name) {
            pick["c"] = json!(true);
        }
        if is_in(&skills, name) {
            pick["s"] = json!(true);
        }
        picks.push(pick);
    }
    picks
}

/// The first T3 whose sends take `freshContext` (T3 Code 0.18.0 reports its server as 0.1.8).
/// MCP doesn't list a server's capabilities, so its version stands in for that one.
const FRESH_CONTEXT_SINCE: [u64; 3] = [0, 1, 8];

/// Whether a T3 that reports this server version starts fresh sessions. A version that
/// doesn't read as numbers (none, from a T3 too old to say) doesn't.
fn takes_fresh_context(version: &str) -> bool {
    let core = version.trim().trim_start_matches('v').split(['-', '+']).next().unwrap_or("");
    let parts: Option<Vec<u64>> = core.split('.').map(|part| part.parse().ok()).collect();
    match parts {
        Some(mut parts) if !parts.is_empty() => {
            parts.resize(3, 0);
            parts[..3] >= FRESH_CONTEXT_SINCE[..]
        }
        _ => false,
    }
}

/// What the game calls the models it knows. T3 labels only some of them (GPT-6.1 Terra has a
/// label, gpt-6.1-sol doesn't), which reads unevenly side by side.
const MODEL_NAMES: [(&str, &str); 10] = [
    ("gpt-6.1-sol", "GPT-6.1 Sol"),
    ("gpt-6.1-terra", "GPT-6.1 Terra"),
    ("gpt-6-sol", "GPT-6 Sol"),
    ("gpt-6-astra", "GPT-6 Astra"),
    ("gpt-6-luna", "GPT-6 Luna"),
    ("gpt-5.6-sol", "GPT-5.6 Sol"),
    ("claude-opus-5-5", "Claude Opus 5.5"),
    ("claude-sonnet-5-5", "Claude Sonnet 5.5"),
    ("claude-fable-5-1", "Claude Fable 5.1"),
    ("claude-haiku-5-5", "Claude Haiku 5.5"),
];

/// A model's name for the game: ours if we know it, else T3's label, else its id, so a model
/// that's new to this version still shows.
fn model_text<'a>(id: &'a str, label: Option<&'a str>) -> &'a str {
    MODEL_NAMES.iter().find(|(known, _)| *known == id).map(|(_, name)| *name).or(label).unwrap_or(id)
}

/// The name a `model:<instance>/<model>` reference reads as, when the machine has that
/// provider and model (`compact_providers`): game markup, or plain text for a table cell.
/// None leaves the reference as written, as T3 does with one it can't resolve.
fn model_ref_name(providers: &[Value], instance: &str, model: &str, cell: bool) -> Option<String> {
    // In game markup the reference has its pipes doubled; the machine's ids don't.
    let model = if cell { model.to_string() } else { model.replace("||", "|") };
    let provider = providers.iter().find(|p| p["id"] == instance)?;
    let entry = provider["m"].as_array()?.iter().find(|m| m["id"] == model.as_str())?;
    let name = str_of(&entry["t"]);
    (!name.is_empty()).then(|| if cell { name.replace("||", "|") } else { name.to_string() })
}

/// What the open thread runs on, for the game: T3's read of its settings, else what the
/// thread's own read says (which has no effort). None when neither names a model.
fn thread_settings(thread: &Value, config: Option<&Value>) -> Option<Value> {
    let selection = config.map(|c| &c["modelSelection"]).filter(|s| s["model"].is_string());
    let model = selection.map_or(str_of(&thread["model"]), |s| str_of(&s["model"]));
    if model.is_empty() {
        return None;
    }
    let provider = selection.map_or(str_of(&thread["providerInstanceId"]), |s| str_of(&s["instanceId"]));
    let mode = config.and_then(|c| c["runtimeMode"].as_str()).unwrap_or(str_of(&thread["runtimeMode"]));
    let mut cfg = json!({ "pv": provider, "m": model, "rm": mode });
    // A model off the machine's short list still has its name, where one is known.
    let name = model_text(model, None);
    if name != model {
        cfg["mt"] = json!(name);
    }
    let options = selection.and_then(|s| s["options"].as_array());
    if let Some(effort) = options.into_iter().flatten().find(|o| is_effort(str_of(&o["id"]))) {
        cfg["ef"] = effort["value"].clone();
    }
    Some(cfg)
}

/// The settings `resolved_scripts` reads.
const SCRIPT_SETTINGS: [&str; 4] = ["projectSettingsOverrides", "projectSettingsFolded", "defaultProjectScripts", "projectScriptOverrides"];

/// A project's scripts as T3's own dropdown works them out (its `resolveProjectScripts`), from
/// T3's settings and the project's own list.
fn resolved_scripts<'a>(settings: &'a Value, project: &str, own: &'a [Value]) -> &'a [Value] {
    fn list(value: &Value) -> &[Value] {
        value.as_array().map_or(&[], Vec::as_slice)
    }
    if let Some(chosen) = settings["projectSettingsOverrides"][project]["defaultProjectScripts"].as_array() {
        return chosen;
    }
    let defaults = list(&settings["defaultProjectScripts"]);
    if settings["projectSettingsFolded"] == true {
        return defaults;
    }
    match settings["projectScriptOverrides"].get(project) {
        Some(Value::Null) => defaults,
        Some(legacy) => list(legacy),
        None if own.is_empty() => defaults,
        None => own,
    }
}

/// The message that asks a thread's agent to run one of its project's saved scripts.
fn script_request(name: &str, command: &str) -> String {
    format!("Run this project's saved script \"{name}\" in this thread's folder:\n\n```\n{command}\n```\n\nIf it keeps running, like a dev server, leave it running in the background. Then tell me how it went.")
}

fn str_of(value: &Value) -> &str {
    value.as_str().unwrap_or("")
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_else(|_| THIS_MACHINE.into())
}

/// The project T3 keeps scratch threads in. New Thread has its own "No project" for those,
/// as T3's own picker does. T3's tools don't mark it, so it's told by having no repository
/// and T3's scratch folder or scratch icon, not by its title, which anyone's project can have.
fn is_scratch_project(project: &Value) -> bool {
    let root = str_of(&project["workspaceRoot"]).replace('\\', "/");
    let scratch_folder = root.trim_end_matches('/').ends_with(".t3/scratch");
    project["repositoryIdentity"].is_null() && (scratch_folder || project["projectIcon"]["name"] == "message-square-dashed")
}

/// The longest snooze the game may ask for: a year.
const MAX_SNOOZE_MINUTES: i64 = 525_600;

/// Epoch seconds to ISO 8601 in UTC ("2026-11-07T00:22:41.000Z"), as T3 writes its times.
fn iso_from_seconds(seconds: i64) -> String {
    let (days, rest) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02}T{:02}:{:02}:{:02}.000Z", rest / 3600, rest % 3600 / 60, rest % 60)
}

/// ISO 8601 ("2026-11-07T00:22:41.923Z", or with an offset) to epoch seconds; 0 if unreadable.
pub fn iso_seconds(iso: &str) -> i64 {
    fn num(s: &str, range: std::ops::Range<usize>) -> Option<i64> {
        s.get(range)?.parse().ok()
    }
    let parse = || -> Option<i64> {
        let (y, mo, d) = (num(iso, 0..4)?, num(iso, 5..7)?, num(iso, 8..10)?);
        let (h, mi, s) = (num(iso, 11..13)?, num(iso, 14..16)?, num(iso, 17..19)?);
        let mut total = days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + s;
        let rest = iso.get(19..)?;
        let zone = rest.trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
        if let Some(sign) = zone.chars().next().filter(|c| *c == '+' || *c == '-') {
            // "+03:00", "+0300" or "+03".
            let digits: String = zone[1..].chars().filter(char::is_ascii_digit).collect();
            let (oh, om) = (num(&digits, 0..2)?, num(&digits, 2..4).unwrap_or(0));
            let offset = oh * 3600 + om * 60;
            total -= if sign == '+' { offset } else { -offset };
        }
        Some(total)
    };
    if iso.is_empty() {
        0
    } else {
        parse().unwrap_or(0)
    }
}

/// Saved between runs in state.json (same shape as the earlier Node companion's).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LinkState {
    pub wow_key: Option<String>,
    pub ui_session: Option<String>,
    pub slot: u32,
    /// Where the game's current wait started: signal file number and epoch seconds.
    pub signal: u32,
    pub signal_time: i64,
    pub acked: BTreeMap<String, u64>,
    pub client_key: Option<String>,
    pub polling: Option<bool>,
    /// The T3 sign-in the game's updates come from (`Config::server_key`), and the stamp
    /// they carry for it (`lid`). The stamp changes with the T3, and on a sign-out: the
    /// game then drops everything it had from the previous one.
    pub login_key: Option<String>,
    pub login: Option<String>,
    /// What each image slot holds on disk (`slots::image_file`).
    pub images: BTreeMap<u32, ImageSlot>,
    /// The image slots named in updates to the game process `wow_key`: it may have loaded
    /// them, and keeps what it loaded until it quits, so they aren't written again until then.
    pub images_shown: Vec<u32>,
    /// The threads New Thread started from the game, oldest first, at most `STARTED_KEPT`.
    /// They start through T3's tools, so T3 records them as an agent's, like the delegated
    /// threads its sidebar leaves out; only this says they're yours.
    pub started_threads: Vec<String>,
    /// The messages the game's /clear sent in a fresh session, oldest first, at most
    /// `FRESH_KEPT`. T3 draws its New session line above such a message, but its read
    /// doesn't say which ones they are (T3 Code 0.19.0), so only this marks them.
    pub fresh_messages: Vec<String>,
}

/// How many threads started from the game are remembered.
const STARTED_KEPT: usize = 500;

/// How many messages sent in a fresh session are remembered.
const FRESH_KEPT: usize = 500;

/// A picture in an image slot.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ImageSlot {
    /// What was drawn (`Picture::key`).
    pub key: String,
    pub w: u32,
    pub h: u32,
    /// When an update last named it (epoch ms).
    pub used: i64,
}

impl Default for LinkState {
    fn default() -> LinkState {
        LinkState {
            wow_key: None,
            ui_session: None,
            slot: 1,
            signal: 1,
            signal_time: 0,
            acked: BTreeMap::new(),
            client_key: None,
            polling: None,
            login_key: None,
            login: None,
            images: BTreeMap::new(),
            images_shown: Vec::new(),
            started_threads: Vec::new(),
            fresh_messages: Vec::new(),
        }
    }
}

/// Commands that mean the same whichever T3 the game thought it was talking to; any
/// other one sent under a previous sign-in's stamp is refused, not run on this T3.
/// Keep in step with LOGIN_FREE in addon/T3WoW/Link.lua.
const LOGIN_FREE: [&str; 4] = ["hello", "ui", "probe", "refresh"];

/// Why such a command was refused, for the game window.
const OTHER_LOGIN: &str = "not sent: T3 WoW has switched to another T3, and this was meant for the previous one";

/// Limits on a command that comes in parts; keep in step with addon/T3WoW/Codec.lua. The
/// JSON limit also caps what a bad frame can make the companion inflate.
const MAX_PARTS: u64 = 16;
const MAX_LONG_JSON: usize = 64_000;
/// Stands in for a long command whose parts joined into nothing readable.
const GARBLED: &str = "garbled";
const GARBLED_REASON: &str = "T3 WoW couldn't read that long message: send it again";
/// Why a command from a frame that isn't signed with this install's key is refused, when
/// it would change T3. Another addon can draw a strip but can't sign it; T3 WoW's own
/// frames go unsigned when the game started before the companion wrote the key.
const UNSIGNED_REASON: &str = "not signed by T3 WoW: restart WoW so it loads the key the companion wrote";

/// The parts seen so far of the long command the game is drawing. The game sends one at a
/// time, so this is all that's kept, whatever another addon draws on the strip.
struct LongCommand {
    from: (String, String, u64),
    zlib: bool,
    parts: Vec<Option<Vec<u8>>>,
    /// Whether every part so far came in a signed frame.
    signed: bool,
}

/// The command in a long command's joined bytes.
fn read_long_command(body: &[u8], zlib: bool, s: u64) -> Value {
    let inflated = if zlib { miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(body, MAX_LONG_JSON).ok() } else { None };
    let text = if zlib { inflated.as_deref() } else { (body.len() <= MAX_LONG_JSON).then_some(body) };
    text.and_then(|text| serde_json::from_slice::<Value>(text).ok())
        .filter(|command| command["s"].as_u64() == Some(s))
        .unwrap_or_else(|| json!({ "s": s, "t": GARBLED }))
}

/// A new stamp for the game's updates (8 hex digits, random).
fn new_login_stamp() -> String {
    let mut bytes = [0u8; 4];
    if getrandom::fill(&mut bytes).is_err() {
        bytes = (crate::config::now_ms() as u32).to_le_bytes();
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub enum CaptureMessage {
    /// The WoW client process changed: Some(key) when one runs, None when none does.
    Wow(Option<String>),
    /// Cell values read from the strip, and when (epoch ms).
    Strip(Vec<u8>, i64),
}

#[derive(Clone)]
struct Env {
    id: String,
    name: String,
    ok: bool,
}

#[derive(Clone)]
struct View {
    env: String,
    id: String,
    depth: usize,
}

/// The open thread's messages as last read, with what decides whether reading on from
/// the last one is enough.
struct ViewCache {
    env: String,
    id: String,
    depth: usize,
    item_count: usize,
    status: String,
    /// When the last full read was.
    read_at: i64,
    messages: Vec<Value>,
    /// There are older messages than these.
    earlier: bool,
}

/// The game's top line while the sign-in in use can only read.
#[cfg(not(target_os = "macos"))]
const READ_ONLY_LINE: &str = "Read-only T3 sign-in: sign in again from the tray menu, pick Auto or Full access";
// A Mac: the menu bar's Sign in to T3…, or under `t3wow run` (no menu) `t3wow login`.
#[cfg(target_os = "macos")]
const READ_ONLY_LINE: (&str, &str) = (
    "Read-only T3 sign-in: choose Sign in to T3… in the menu bar, pick Auto or Full access",
    "Read-only T3 sign-in: run t3wow login again, pick Auto or Full access",
);

/// What the tray shows.
#[derive(Clone, Debug, Default)]
pub struct BridgeStatus {
    pub t3_ok: bool,
    pub t3_problem: Option<String>,
    pub auth_needed: bool,
    /// The sign-in in use can only read (`Bridge::set_sign_in_read_only`).
    pub sign_in_read_only: bool,
    pub wow_running: bool,
    pub in_game: bool,
    pub signals: bool,
    pub machines: usize,
    /// The next slot the game loads (1..=200; past 200 it has used them all).
    pub slot: u32,
    /// This game process used up its signal files: updates come on a timer until WoW restarts.
    pub signals_out: bool,
    /// The signal files went missing from the addon folder: restart WoW once they're back.
    pub signals_lost: bool,
    /// Writing the game's update files keeps failing.
    pub write_problem: Option<String>,
    /// The T3 the companion talks to runs an older T3 Code than this release is made for.
    pub older_t3: Option<OlderT3>,
}

/// After this many failed publishes in a row, the tray shows the error.
const WRITE_FAILURES_SHOWN: u32 = 3;

pub struct Bridge {
    client: Arc<dyn T3>,
    slots: Box<dyn SlotWriter>,
    state_file: Option<PathBuf>,
    log: Box<dyn Fn(&str) + Send>,
    now: Box<dyn Fn() -> i64 + Send>,
    /// Opens a link with Windows (T3's t3code:// links); replaced in tests.
    pub opener: Box<dyn Fn(&str) + Send>,
    /// This PC's offset from UTC at an instant, for snooze times on its clock; replaced in
    /// tests.
    pub zone: Box<dyn Fn(i64) -> i64 + Send>,
    pub link: LinkState,
    /// The capture has said whether WoW runs. Until then `link.wow_key` is only what the last
    /// run saw, and that game may have quit since.
    wow_seen: bool,
    window_open: bool,
    view: Option<View>,
    envs: Vec<Env>,
    local_env_id: Option<String>,
    /// Per machine: its sidebar threads, its projects and how many threads it has settled.
    threads: BTreeMap<String, Vec<Value>>,
    projects: BTreeMap<String, Vec<Value>>,
    /// Per machine: whether its new threads start in a new worktree by default (T3's
    /// "New thread mode" preference), for projects that don't set their own.
    new_worktrees: BTreeMap<String, bool>,
    /// The machines whose T3 can start a fresh provider session in a thread (`freshContext`
    /// on a send, T3's /clear), by the version each one reports.
    fresh_context: HashSet<String>,
    /// The T3 the companion talks to runs an older T3 Code than this release is made for (by
    /// the version it reports): the tray and the game suggest updating it.
    older_t3: Option<OlderT3>,
    /// What each machine offers for the model and effort selectors.
    catalogs: BTreeMap<String, Catalog>,
    /// T3's folder of provider caches, while the T3 in use is the one on this PC: MCP doesn't
    /// list skills and commands, and only this PC's are on this disk.
    provider_caches: Option<PathBuf>,
    /// T3's settings file, while the T3 in use is the one on this PC.
    t3_settings: Option<PathBuf>,
    /// What it last said about scripts (`read_t3_settings`).
    script_settings: Option<Value>,
    /// And its snooze presets (Settings > Snooze options), if it lists any.
    snooze_presets: Option<Vec<Value>>,
    settled_totals: BTreeMap<String, u64>,
    /// Per machine: which of its three lists have more threads than were listed.
    list_more: BTreeMap<String, [bool; LIST_COUNT]>,
    /// How many threads each of the three lists asks for; the game's Show more raises them
    /// for the UI session.
    list_limits: [usize; LIST_COUNT],
    view_data: Option<Value>,
    view_key: Option<String>,
    view_cache: Option<ViewCache>,
    /// Counts commands that changed what the open thread shows (see `view_changed`).
    view_gen: u64,
    notes: Vec<Value>,
    events: Vec<Value>,
    event_seq: i64,
    last_status: HashMap<String, String>,
    /// The requests each thread waiting on you has notified for, until it stops waiting.
    shown_requests: HashMap<String, HashSet<String>>,
    /// Active threads off the lists whose last reads failed, and how many times in a row.
    unlisted_misses: HashMap<String, u32>,
    /// What working threads wait on, keyed by thread: T3's list says "running" while an
    /// agent waits on a question or an approval, and only a thread read has the count.
    pending_counts: HashMap<String, Pending>,
    /// Whether threads with an open pull request watch it, keyed by thread: only T3's
    /// pull request list says so.
    watches: HashMap<String, Watch>,
    /// Where each agent-made thread came from, read once (again on a change, while undecided).
    launchers: HashMap<String, Origin>,
    /// Per machine, the threads the last read found waiting on a subagent.
    delegating: HashMap<String, HashSet<String>>,
    /// Each working thread's current run and when it started, read once per run: T3's
    /// lists don't say.
    run_starts: HashMap<String, (String, i64)>,
    /// Each thread at rest's last activity (epoch seconds), by the updatedAt it was read at.
    activity: HashMap<String, (String, i64)>,
    /// Machines whose threads were listed at least once since the companion started.
    listed_envs: HashSet<String>,
    /// Commands waiting to run: client key, the UI session they came in, the stamp of the
    /// sign-in the game drew them under (none from an older addon), the command, when the
    /// frame that brought it was read, and whether that frame was signed.
    commands: VecDeque<(String, String, Option<String>, Value, i64, bool)>,
    /// Per client key, the highest command queued to run but not yet run (not saved: a
    /// command only counts as done once it ran).
    queued: HashMap<String, u64>,
    last_publish_at: i64,
    last_published_hash: Option<u64>,
    last_urgent_hash: Option<u64>,
    /// The parts the game holds, by hash (see KEPT_PARTS): all of the latest update's once
    /// a frame says the game is in step with it (g), and a part goes as soon as an update
    /// carries another one, until the game is in step again. So a part the game had can
    /// be sent again, but one it didn't have is never left out.
    held: BTreeMap<&'static str, String>,
    /// The latest update's number (g in it) and the hashes of its parts.
    update_id: u32,
    update_parts: BTreeMap<&'static str, String>,
    /// When the next poll starts, and the one under way (if any).
    poll_at: i64,
    poll_run: Option<PollRun>,
    /// A command ran during a poll: poll again once it ends.
    repoll: bool,
    /// Failed polls in a row, which space the next ones out.
    poll_failures: u32,
    /// The last thread-list error per machine, logged once.
    list_errors: HashMap<String, String>,
    slow_at: i64,
    auth_error: Option<String>,
    /// T3 granted the sign-in in use only Read only access: the game's top line says so.
    sign_in_read_only: bool,
    poll_error: Option<String>,
    polled_ok: bool,
    /// The question shown in the game for the open thread, with the ids T3 needs back.
    pending_ask: Option<PendingAsk>,
    /// Outcomes of this UI session's recent commands, with when they ran.
    outcomes: Vec<(i64, Value)>,
    /// Threads T3 is stopping at the game's word ("env:id"), with when it was asked: the
    /// next list that shows one no longer working says "Stopped.".
    stopping: HashMap<String, i64>,
    /// Why signals stopped (this game process used every signal file, or they can't be
    /// written), until WoW restarts.
    signals_out: Option<String>,
    /// Publishes in a row whose update files were written but whose signals weren't.
    signal_failures: u32,
    /// The UI session whose running out of slots was logged.
    slots_out_logged: Option<String>,
    /// The last failure writing update files, and how many publishes in a row failed.
    write_error: Option<String>,
    write_failures: u32,
    /// The tray's "Commands from the game": what the game may make T3 do.
    trust: Trust,
    /// The key the addon signs its frames with (Key.lua); None (config.json couldn't be
    /// read or saved): nothing counts as signed, so the game can only read.
    game_key: Option<Vec<u8>>,
    /// The game's latest frame wasn't signed with the key: it gets only what Read-only
    /// allows, and the window says to restart WoW.
    unsigned: bool,
    /// Draws pictures for the game (None: no browser to draw with, so no pictures).
    painter: Option<Box<dyn Painter>>,
    /// The game's addon can show pictures this UI session (its frames say `im`).
    images_ok: bool,
    /// The game's addon draws what an agent did as collapsed rows (its frames say `wk`).
    work_ok: bool,
    /// The rows opened most recently, until the game has had them (it keeps them after).
    details: Vec<Detail>,
    /// The game's addon draws delegated tasks as cards (its frames say `dc`).
    cards_ok: bool,
    /// The open thread's pages found so far (see `read_view_scan`).
    scan: Option<ScanCache>,
    /// Pictures that couldn't be drawn, why, and when (tried again after PICTURE_RETRY_MS).
    picture_errors: HashMap<String, (String, i64)>,
    /// Reads the image on the clipboard (`clip::read_image`); replaced in tests.
    pub clipboard: Box<dyn Fn() -> Result<Option<Clipped>, String> + Send>,
    /// Ctrl+V presses seen in WoW, and when the last was (`keys::presses`); replaced in tests.
    pub presses: Box<dyn Fn() -> (u64, i64) + Send>,
    /// Turns the look for Ctrl+V on or off (`keys::arm`); replaced in tests.
    pub arm_keys: Box<dyn Fn(bool) + Send>,
    /// Whether macOS may be hiding the keys, so no Cmd+V can be seen (`keys::keys_hidden`;
    /// never elsewhere); replaced in tests.
    pub keys_hidden: Box<dyn Fn() -> bool + Send>,
    /// The presses pastes have used: each press lets one paste through.
    presses_used: u64,
    /// Images pasted in the game, by the message they go with: "new" for New Thread,
    /// "{e}:{id}" for a thread, as the game's commands name its machine and id.
    drafts: BTreeMap<String, Vec<Pasted>>,
    /// Images sent from the game whose message T3 hasn't shown yet, oldest first.
    sent: Vec<SentImages>,
    pasted_count: u64,
    /// When this companion started (epoch ms), so its pasted images' small pictures don't
    /// take the image slot keys of an earlier run's.
    started_ms: i64,
    /// When the frame that brought the command being run was read (epoch ms).
    command_seen_at: i64,
    long_command: Option<LongCommand>,
}

/// The pages an agent showed (html_render) in the open thread, found by reading its
/// activity from the first message shown on.
struct ScanCache {
    env: String,
    id: String,
    /// The first position looked at, and the last one read past.
    from: u64,
    upto: Option<u64>,
    pages: Vec<FoundPage>,
    cards: Vec<FoundCard>,
    /// Where the thread's context was compacted (T3 draws a line there).
    compactions: Vec<u64>,
    /// The files each finished turn changed.
    changes: Vec<FoundChanges>,
    /// What it did between its messages, by position.
    work: Vec<FoundWork>,
    /// The run each item read belongs to and when it was last touched, by position: what
    /// a finished turn folds, and for how long a steered part of it went on.
    runs: BTreeMap<u64, (String, i64)>,
}

/// What an activity item did, for the row T3 folds it into.
#[derive(Clone, Copy, PartialEq)]
enum WorkKind {
    Thought,
    Question,
    Read,
    Edit,
    Command,
    Search,
    Web,
    Tool,
}

/// Something the agent did between its messages, which T3 draws as a collapsed row.
#[derive(Clone)]
struct FoundWork {
    position: u64,
    kind: WorkKind,
    /// A thought's or question's start, or T3's title for a tool (many have none).
    label: String,
    /// False: still under way, so the scan comes back for it.
    ended: bool,
    /// A question: what you answered (its tool call is read once for that), and when.
    answer: Option<String>,
    answered: bool,
    at: i64,
}

impl FoundWork {
    fn from_item(item: &Value, position: u64, now: i64) -> Option<FoundWork> {
        let text = str_of(&item["text"]);
        let kind = match str_of(&item["type"]) {
            "reasoning" => WorkKind::Thought,
            "user_input_request" => WorkKind::Question,
            "command_execution" => WorkKind::Command,
            "file_change" => WorkKind::Edit,
            "web_search" => WorkKind::Web,
            "dynamic_tool" => match tool_name(text) {
                "Read" => WorkKind::Read,
                "Edit" | "Write" | "NotebookEdit" => WorkKind::Edit,
                "Bash" | "PowerShell" => WorkKind::Command,
                "Grep" | "Glob" | "ToolSearch" => WorkKind::Search,
                "WebSearch" | "WebFetch" => WorkKind::Web,
                _ => WorkKind::Tool,
            },
            _ => return None,
        };
        // A question can wait for you as long as it likes: its answer is looked for once it ends.
        let stale = kind != WorkKind::Question && now - iso_seconds(str_of(&item["updatedAt"])) * 1000 >= WORK_STALE_MS;
        let ended = stale || matches!(str_of(&item["status"]), "completed" | "failed" | "declined" | "cancelled" | "interrupted" | "error");
        let label = match kind {
            WorkKind::Thought => text.replace("**", ""),
            // Still waiting: its card, further down, has the question.
            WorkKind::Question if !ended => "Input requested".to_string(),
            WorkKind::Question => question_start(text),
            _ => str_of(&item["title"]).to_string(),
        };
        Some(FoundWork { position, kind, label: plain(&label, WORK_LABEL_CHARS), ended, answer: None, answered: false, at: iso_seconds(str_of(&item["updatedAt"])) })
    }

    fn is_tool(&self) -> bool {
        !matches!(self.kind, WorkKind::Thought | WorkKind::Question)
    }
}

/// The tool a dynamic_tool item ran, from its text (cut short, so not always valid JSON).
fn tool_name(text: &str) -> &str {
    text.split_once("\"toolName\": \"").and_then(|(_, rest)| rest.split_once('"')).map_or("", |(name, _)| name)
}

/// The start of a question from a user_input_request's text (JSON, cut short). A question
/// with no words to show (an id that is a slug) is just said to have been asked.
fn question_start(text: &str) -> String {
    let start = text.split_once("\"question\": \"").or_else(|| text.split_once("\"id\": \"")).map_or("", |(_, rest)| rest);
    let mut out = String::new();
    let mut chars = start.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => break,
            '\\' => match chars.next() {
                Some('n') => out.push(' '),
                Some(escaped) => out.push(escaped),
                None => break,
            },
            c => out.push(c),
        }
    }
    if out.contains(' ') { out } else { "Asked a question".to_string() }
}

/// A row's sentence: T3's title for a single tool that has one, else what the tools did.
fn work_summary(rows: &[&FoundWork]) -> String {
    if let [only] = rows {
        if !only.label.is_empty() {
            return only.label.clone();
        }
    }
    let phrases: Vec<String> = [
        (WorkKind::Read, "read", "file", "files"),
        (WorkKind::Edit, "edited", "file", "files"),
        (WorkKind::Command, "ran", "command", "commands"),
        (WorkKind::Search, "ran", "search", "searches"),
        (WorkKind::Tool, "used", "tool", "tools"),
    ]
    .iter()
    .filter_map(|(kind, verb, one, many)| {
        // Searching the code and the web read the same here (only their icons differ).
        let n = rows.iter().filter(|r| r.kind == *kind || (*kind == WorkKind::Search && r.kind == WorkKind::Web)).count();
        (n > 0).then(|| format!("{verb} {n} {}", if n == 1 { one } else { many }))
    })
    .collect();
    let sentence = match phrases.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [one, two] => format!("{one} and {two}"),
        [rest @ .., last] => format!("{}, and {last}", rest.join(", ")),
    };
    let mut letters = sentence.chars();
    letters.next().map_or(String::new(), |first| first.to_uppercase().chain(letters).collect())
}

/// The rows the game shows, each with where it goes among the view's items. Tools in a row
/// fold into one, until a thought, a question or something shown (`walls`) comes between.
/// `done_before`: a thread still working shows only what its last message came after, so a
/// stretch of work reaches the game once it is finished and costs no updates of its own.
fn work_rows(work: &[FoundWork], first: u64, done_before: Option<u64>, walls: &[u64]) -> Vec<(u64, Value)> {
    // k: its icon, for what all of it was; a mix of tools is just tools.
    fn row(rows: &[&FoundWork], from: u64, text: String, out: &mut Vec<(u64, Value)>) {
        let last = rows[rows.len() - 1].position;
        let icon = match rows[0].kind {
            _ if rows.iter().any(|r| r.kind != rows[0].kind) => "t",
            WorkKind::Thought => "b",
            WorkKind::Read => "r",
            WorkKind::Edit => "e",
            WorkKind::Command => "c",
            WorkKind::Search => "s",
            WorkKind::Web => "w",
            _ => "t",
        };
        out.push((rows[0].position * 2, json!({ "r": "a", "x": text, "w": { "p": from, "l": last, "k": icon } })));
    }
    fn flush(tools: &mut Vec<&FoundWork>, out: &mut Vec<(u64, Value)>) {
        if !tools.is_empty() {
            row(tools, tools[0].position, work_summary(tools), out);
            tools.clear();
        }
    }
    let mut out = Vec::new();
    let mut tools: Vec<&FoundWork> = Vec::new();
    for item in work.iter().filter(|w| w.position >= first && done_before.is_none_or(|p| w.position < p)) {
        if item.is_tool() {
            let walled = tools.last().is_some_and(|prev| walls.iter().any(|w| *w > prev.position && *w < item.position));
            if walled || tools.first().is_some_and(|start| item.position - start.position >= WORK_ROW_SPAN) {
                flush(&mut tools, &mut out);
            }
            tools.push(item);
            continue;
        }
        flush(&mut tools, &mut out);
        if !item.label.is_empty() {
            // A question's answer is in the tool call just before it, read along with it.
            let from = if item.kind == WorkKind::Question { item.position.saturating_sub(1) } else { item.position };
            row(&[item], from, item.label.clone(), &mut out);
            if item.kind == WorkKind::Question {
                if let Some((_, shown)) = out.last_mut() {
                    if item.ended {
                        shown["w"]["q"] = json!({ "a": item.answer, "at": item.at });
                    } else {
                        shown["w"]["k"] = json!("q");
                        shown["w"]["still"] = json!(1);
                    }
                }
            }
        }
    }
    flush(&mut tools, &mut out);
    out
}

/// A length of time as T3 says it: "22s", "12m 0s", "1h", "1h 5s". Seconds show
/// whenever minutes do.
fn format_duration(seconds: i64) -> String {
    if seconds < 60 {
        return format!("{seconds}s");
    }
    let (h, m, s) = (seconds / 3600, seconds % 3600 / 60, seconds % 60);
    let parts = [(h, "h", h > 0), (m, "m", m > 0), (s, "s", s > 0 || m > 0)];
    parts.iter().filter(|(_, _, shown)| *shown).map(|(n, unit, _)| format!("{n}{unit}")).collect::<Vec<_>>().join(" ")
}

/// Folds each finished turn under a "Worked for" row, by T3's rules: all it did goes
/// inside but its last message, its questions, whatever isn't a message, row or card (a
/// picture, a compaction's line) and a task still running. Nothing ending badly folds. A
/// folded item says `f`, the name of its row (whose `w.l` is the same). The turn still
/// going is headed by a row saying since when (`w.live`); the game keeps its clock. A
/// message of yours sent into a turn under way (a steer) splits it: what came after folds
/// under its own "Continued for" row, below that message.
fn fold_turns(all: &mut Vec<(u64, Value)>, positions: &[u64], run_at: &BTreeMap<u64, (String, i64)>, recent: &Value) {
    struct Turn<'a> {
        run: &'a str,
        members: Vec<usize>,
        /// Where each steer falls among the members, and when it was sent.
        steers: Vec<(usize, i64)>,
    }
    let mut turns: Vec<Turn> = Vec::new();
    for (i, (order, item)) in all.iter().enumerate() {
        let Some((run, at)) = run_at.get(&(order / 2)) else { continue };
        let known = turns.iter_mut().find(|t| t.run == run.as_str());
        if item["r"] == "u" {
            // T3's read doesn't say it's a steer, but the message that starts a run comes
            // before all it did. Back-to-back steers split once, after the last of them.
            if let Some(turn) = known.filter(|t| !t.members.is_empty() && order.is_multiple_of(2)) {
                if turn.steers.last().is_some_and(|(start, _)| *start == turn.members.len()) {
                    turn.steers.pop();
                }
                turn.steers.push((turn.members.len(), *at));
            }
            continue;
        }
        match known {
            Some(turn) => turn.members.push(i),
            None => turns.push(Turn { run, members: vec![i], steers: Vec::new() }),
        }
    }
    let mut folds: Vec<(usize, Value, Vec<usize>)> = Vec::new();
    let latest = recent.as_array().and_then(|runs| runs.first()).filter(|r| working(str_of(&r["status"])));
    if let Some(latest) = latest {
        let started = iso_seconds(str_of(if latest["startedAt"].is_string() { &latest["startedAt"] } else { &latest["requestedAt"] }));
        if started > 0 {
            let at = turns.iter().find(|t| latest["runId"] == t.run).map_or(all.len(), |t| t.members[0]);
            folds.push((at, json!({ "r": "a", "x": "Working…", "w": { "l": "live", "live": started, "still": 1 } }), Vec::new()));
        }
    }
    for Turn { run, members, steers } in turns {
        let Some(info) = recent.as_array().and_then(|runs| runs.iter().find(|r| r["runId"] == run && r["status"] == "completed")) else { continue };
        let is_message = |i: usize| all[i].0.is_multiple_of(2) && positions.contains(&(all[i].0 / 2));
        let last_message = members.iter().rposition(|i| is_message(*i));
        let hidden: Vec<usize> = members
            .iter()
            .enumerate()
            .filter(|(at, i)| {
                let item = &all[**i].1;
                let (row, card) = (item["w"].is_object(), item["d"].is_object());
                let inside = (is_message(**i) || row || card) && !item["w"]["q"].is_object() && item["d"]["s"] != "running";
                Some(*at) != last_message && inside && (row || last_message.is_none_or(|last| *at < last))
            })
            .map(|(_, i)| *i)
            .collect();
        if hidden.is_empty() {
            continue;
        }
        let seconds = iso_seconds(str_of(&info["completedAt"])) - iso_seconds(str_of(&info["startedAt"]));
        let starts: Vec<usize> = std::iter::once(0).chain(steers.iter().map(|(start, _)| *start)).collect();
        for (n, start) in starts.iter().enumerate() {
            let part = &members[*start..starts.get(n + 1).copied().unwrap_or(members.len())];
            let inside: Vec<usize> = hidden.iter().copied().filter(|i| part.contains(i)).collect();
            if inside.is_empty() {
                continue;
            }
            // The first row keeps the whole turn's time, as T3's does.
            let label = if n == 0 {
                if seconds > 0 { format!("Worked for {}", format_duration(seconds)) } else { "Worked".to_string() }
            } else {
                // A row stands for every tool it folds, up to its last position.
                let spans = part.iter().map(|i| (all[*i].0 / 2, all[*i].1["w"]["l"].as_u64().unwrap_or(all[*i].0 / 2)));
                let ended = spans.flat_map(|(from, to)| run_at.range(from..=to)).map(|(_, (_, at))| *at).max().unwrap_or(0);
                let since = ended - steers[n - 1].1;
                if since > 0 { format!("Continued for {}", format_duration(since)) } else { "Continued".to_string() }
            };
            let name = format!("f{}", all[part[0]].0);
            folds.push((part[0], json!({ "r": "a", "x": label, "w": { "l": name, "fold": 1 } }), inside));
        }
    }
    // From the last: placing a row moves what comes after it.
    folds.sort_by_key(|(first, ..)| std::cmp::Reverse(*first));
    for (first, row, hidden) in folds {
        for i in hidden {
            all[i].1["f"] = row["w"]["l"].clone();
        }
        let order = all.get(first).map_or(u64::MAX, |(order, _)| *order);
        all.insert(first, (order, row));
    }
}

/// A row opened in the game: all it did, in the pieces a message with code is drawn from.
struct Detail {
    env: String,
    id: String,
    last: u64,
    body: Vec<Value>,
    failed: bool,
}

/// What a tool call took, as text: its command or path when it has one, else all of it.
fn tool_input(input: &Value) -> String {
    input["command"].as_str().or(input["file_path"].as_str()).map(str::to_string).unwrap_or_else(|| serde_json::to_string_pretty(input).unwrap_or_default())
}

/// What a tool call gave back, as text.
fn tool_output(output: &Value) -> String {
    let text = output.as_str().or(output["content"].as_str()).or(output["stdout"].as_str()).or(output["file"]["content"].as_str()).or(output["text"].as_str());
    match text {
        Some(text) => text.to_string(),
        None if output.is_null() => String::new(),
        None => serde_json::to_string_pretty(output).unwrap_or_default(),
    }
}

/// Text in code boxes, DETAIL_PIECE characters or so each, from what `left` allows.
fn push_code(out: &mut Vec<Value>, left: &mut usize, text: &str) {
    let mut piece = String::new();
    let mut length = 0;
    for c in text.trim_end().chars().take(*left) {
        piece.push(c);
        length += 1;
        *left -= 1;
        if length >= DETAIL_PIECE && (c == '\n' || length >= DETAIL_PIECE + 500) {
            out.push(json!({ "c": std::mem::take(&mut piece) }));
            length = 0;
        }
    }
    if !piece.trim().is_empty() {
        out.push(json!({ "c": piece }));
    }
}

/// The questions of a row, each with its options and what you answered.
fn question_segments(questions: &[Value], answers: &Value) -> Vec<Value> {
    questions
        .iter()
        .map(|question| {
            let asked = question["question"].as_str().or(question["id"].as_str()).unwrap_or("");
            let mut text = wow_text(asked, 2000);
            for option in question["options"].as_array().into_iter().flatten() {
                text.push_str(&format!("\n• {}", plain(str_of(&option["label"]), 200)));
            }
            if let Some(answer) = answers[asked].as_str() {
                text.push_str(&format!("\n\nYou answered: {}", plain(answer, 2000)));
            }
            json!(text)
        })
        .collect()
}

/// What a row's items did, for the game: DETAIL_CHARS at most.
fn detail_segments(items: &[Value]) -> Vec<Value> {
    let call_of = |item: &Value| serde_json::from_str::<Value>(str_of(&item["text"])).ok().filter(|call| call["toolName"].is_string());
    if let Some(request) = items.iter().find(|item| item["type"] == "user_input_request") {
        let asked = items.iter().filter_map(call_of).find(|call| call["toolName"] == "AskUserQuestion" && call["output"]["questions"].is_array());
        if let Some(call) = asked {
            return question_segments(call["output"]["questions"].as_array().map(Vec::as_slice).unwrap_or_default(), &call["output"]["answers"]);
        }
        let questions = serde_json::from_str::<Value>(str_of(&request["text"])).unwrap_or(Value::Null);
        return question_segments(questions.as_array().map(Vec::as_slice).unwrap_or_default(), &Value::Null);
    }
    let mut out = Vec::new();
    let mut left = DETAIL_CHARS;
    for item in items {
        if left == 0 {
            break;
        }
        let text = str_of(&item["text"]);
        match str_of(&item["type"]) {
            "reasoning" => {
                let thought = wow_text(&text.replace("**", ""), left);
                left = left.saturating_sub(thought.chars().count());
                out.push(json!(thought));
            }
            "command_execution" | "file_change" | "web_search" => push_code(&mut out, &mut left, text),
            "dynamic_tool" => {
                let title = str_of(&item["title"]);
                let heading = if title.trim().is_empty() { tool_name(text) } else { title };
                out.push(json!(plain(heading, 200)));
                match call_of(item) {
                    Some(call) => {
                        push_code(&mut out, &mut left, &tool_input(&call["input"]));
                        push_code(&mut out, &mut left, &tool_output(&call["output"]));
                    }
                    None => push_code(&mut out, &mut left, text),
                }
            }
            _ => {}
        }
    }
    if left == 0 {
        out.push(json!("… the rest is in T3"));
    }
    out
}

/// A task the open thread delegated, as its activity shows it (T3 draws it as a card).
#[derive(Clone)]
struct FoundCard {
    position: u64,
    item_id: String,
    title: String,
    /// As the game names a thread's status: "running", "completed", "failed" or "cancelled".
    status: &'static str,
    /// Its result so far; whether that was cut short, and whether it was read in full
    /// (CARD_CHARS of it) after the task ended.
    text: String,
    truncated: bool,
    full: bool,
    at: i64,
}

impl FoundCard {
    fn from_item(item: &Value, position: u64) -> FoundCard {
        let status = match str_of(&item["status"]) {
            "completed" => "completed",
            "failed" | "error" => "failed",
            "cancelled" | "interrupted" | "declined" => "cancelled",
            _ => "running",
        };
        FoundCard {
            position,
            item_id: str_of(&item["itemId"]).to_string(),
            title: str_of(&item["title"]).to_string(),
            status,
            text: str_of(&item["text"]).to_string(),
            truncated: item["textTruncated"] == true,
            full: false,
            at: iso_seconds(str_of(&item["updatedAt"])),
        }
    }

    fn ended(&self) -> bool {
        self.status != "running"
    }

    /// The task's own part of its id ("delegated-task:..."), which its thread's id
    /// ("thread:delegated-task:...") shares.
    fn task(&self) -> &str {
        task_part(&self.item_id)
    }

    /// The card for the game: its title, status, when it ended and the start of its
    /// result. An addon without cards shows `x`, which says the same in text.
    fn shown(&self) -> Value {
        let title = if self.title.trim().is_empty() { "Delegated task".to_string() } else { plain(&self.title, 120) };
        let body = if self.ended() && !self.text.trim().is_empty() {
            let mut body = wow_text(&self.text, CARD_BODY_CHARS);
            if self.truncated && !body.ends_with('…') {
                body.push_str("\n…");
            }
            body
        } else {
            String::new()
        };
        let word = match self.status {
            "completed" => "finished",
            "failed" => "failed",
            "cancelled" => "stopped",
            _ => "working",
        };
        let x = if body.is_empty() { format!("{title} ({word})") } else { format!("{title} ({word})\n{body}") };
        json!({ "r": "a", "x": x, "d": { "t": title, "s": self.status, "at": self.at, "b": body } })
    }
}

/// What an update would show, for telling whether anything changed. A thread being busy
/// moves its time and its place in the list, which alone isn't worth one of the 200 slots:
/// the next update with anything to show brings them.
fn without_times(data: &Value) -> Value {
    let mut data = data.clone();
    if let Some(threads) = data["threads"].as_array_mut() {
        threads.iter_mut().filter_map(Value::as_object_mut).for_each(|t| {
            t.remove("at");
        });
        threads.sort_by_key(|t| format!("{}:{}", t["e"], t["id"]));
    }
    if let Some(view) = data["view"].as_object_mut() {
        view.remove("at");
    }
    data
}

/// The part of a T3 id after its kind ("turn-item:", "thread:", "node:").
fn task_part(id: &str) -> &str {
    id.split_once(':').map_or(id, |(_, rest)| rest)
}

#[derive(Clone)]
struct FoundPage {
    position: u64,
    item_id: String,
    title: String,
}

/// The files a turn changed: T3's checkpoint item at the end of a run, which T3 shows as
/// a card under the turn's last message. Each file is (path, lines added, lines removed).
#[derive(Clone)]
struct FoundChanges {
    position: u64,
    item_id: String,
    files: Vec<(String, u64, u64)>,
    /// The text read stopped short: there are more files than `files`.
    cut: bool,
    /// Read as far as it will be (the scan reads only the start of a long list).
    full: bool,
}

/// Files a turn's card lists by name; the rest are counted, so a big diff stays a few lines.
const CHANGED_FILES_SHOWN: usize = 6;
/// How much of a checkpoint's file list is read (about 400 files).
const CHANGES_CHARS: usize = 50_000;

/// A checkpoint item's files, from its text: T3's pretty-printed JSON list. Text cut short
/// keeps the files it holds whole.
fn changed_files(text: &str, truncated: bool) -> Vec<(String, u64, u64)> {
    let whole = match (truncated, text.rfind("\n  }")) {
        (false, _) => text.to_string(),
        (true, Some(end)) => format!("{}\n]", &text[..end + 4]),
        (true, None) => return Vec::new(),
    };
    let files: Vec<Value> = serde_json::from_str(&whole).unwrap_or_default();
    files
        .iter()
        .filter_map(|file| {
            let path = file["path"].as_str()?.trim();
            (!path.is_empty()).then(|| (path.to_string(), file["additions"].as_u64().unwrap_or(0), file["deletions"].as_u64().unwrap_or(0)))
        })
        .collect()
}

/// A count as T3's diff stats write it: 999, 1.2k, 12k, 1.5m.
fn compact_count(n: u64) -> String {
    let short = |value: f64, unit: &str| {
        let text = if value < 10.0 { format!("{value:.1}") } else { format!("{}", value.round()) };
        format!("{}{unit}", text.strip_suffix(".0").unwrap_or(&text))
    };
    match n {
        0..1000 => n.to_string(),
        1000..999_500 => short(n as f64 / 1e3, "k"),
        _ => short(n as f64 / 1e6, "m"),
    }
}

impl FoundChanges {
    /// The card for the game: `cf` for an addon that draws it, and the same as plain lines
    /// for one that doesn't (and for copying).
    fn shown(&self) -> Value {
        let count = self.files.len();
        let (added, removed) = self.files.iter().fold((0, 0), |(a, d), (_, fa, fd)| (a + fa, d + fd));
        let plus = if self.cut { "+" } else { "" };
        let mut head = format!("{count}{plus} changed file{}", if count == 1 && !self.cut { "" } else { "s" });
        // Counts over part of the list would say less than the turn did.
        if !self.cut && (added > 0 || removed > 0) {
            head = format!("{head}  +{} -{}", compact_count(added), compact_count(removed));
        }
        let listed = &self.files[..count.min(CHANGED_FILES_SHOWN)];
        let more = count - listed.len();
        let mut lines = vec![head];
        lines.extend(listed.iter().map(|(path, a, d)| format!("{}  +{} -{}", plain(path, 200), compact_count(*a), compact_count(*d))));
        if more > 0 {
            lines.push(format!("{more}{plus} more file{}", if more == 1 && !self.cut { "" } else { "s" }));
        }
        let files: Vec<Value> = listed.iter().map(|(path, a, d)| json!({ "p": plain(path, 200), "a": a, "d": d })).collect();
        let mut cf = json!({ "n": count, "f": files, "more": more });
        if self.cut {
            cf["cut"] = json!(1);
        } else {
            cf["a"] = json!(added);
            cf["d"] = json!(removed);
        }
        json!({ "r": "a", "x": lines.join("\n"), "cf": cf })
    }
}

/// A picture the open thread shows: a page an agent showed, or an image a message embeds
/// from a file on this PC.
#[derive(Clone)]
enum Source {
    Page { item_id: String },
    File { path: PathBuf },
    /// Already in an image slot: an image sent from the game, which can't be drawn again.
    Sent,
}

#[derive(Clone)]
struct Picture {
    /// What it is, for finding it in an image slot again: the item (or the file as it is
    /// now) and the way pages are drawn.
    key: String,
    source: Source,
    /// The page's title or the image's alt text.
    label: String,
    /// Where it goes among the shown items: twice the position of the item it comes from,
    /// plus one for an image that follows its message.
    order: u64,
    role: &'static str,
}

/// A picture being drawn: the page's item text read so far, a piece at a time.
struct Drawing {
    picture: Picture,
    text: String,
    offset: Option<u64>,
    pieces: usize,
}

/// Pictures drawn in one poll at most (each takes the browser a second or two).
const DRAWS_PER_POLL: usize = 2;
/// Delegated tasks read again in one poll at most (those still running, and those that
/// ended with more result than the scan reads).
const CARD_READS_PER_POLL: usize = 4;
/// How much of a delegated task's result is read, and shown on its card.
const CARD_CHARS: usize = 600;
const CARD_BODY_CHARS: usize = 400;
/// Activity pages read in one poll at most while looking for pages.
const SCAN_PAGES: usize = 4;
/// How long a row's one line can be.
const WORK_LABEL_CHARS: usize = 140;
/// A tool call still "running" this long was left behind: it's treated as ended.
const WORK_STALE_MS: i64 = 600_000;
/// How much of a row is read and sent when it is opened, how long each code box of it is
/// about, and how many opened rows are kept for the game.
const DETAIL_CHARS: usize = 30_000;
const DETAIL_PIECE: usize = 4_000;
const DETAILS_KEPT: usize = 3;
/// A row covers at most this many positions: all T3 returns in the one read that opens it.
const WORK_ROW_SPAN: u64 = 100;
/// A picture that couldn't be drawn is tried again after this long.
const PICTURE_RETRY_MS: i64 = 300_000;
/// A page's item is read a piece at a time, this many pieces at most (html_render takes
/// 512 KB of HTML).
const PAGE_TEXT_PIECES: usize = 16;
/// Pictures kept on disk when a new game process starts (the rest give their space back).
const IMAGES_KEPT: usize = 40;
/// How the key of an image sent from the game starts: nothing can draw one again.
const SENT_KEY: &str = "sent:";
/// Said in place of a picture once every image slot holds one this game process has seen.
const IMAGES_OUT: &str = "no picture slots left in this WoW session: restart WoW to see more";

/// Whether a thread watches its pull request, as last read.
#[derive(Clone, Debug)]
struct Watch {
    /// The thread's `updatedAt` when read, and when that was.
    updated: String,
    read_at: i64,
    watching: bool,
}

/// Images pasted for one message at most (T3 takes 8).
const MAX_PASTED: usize = 8;
/// A paste counts only if the frame that brought it was read this soon after a Ctrl+V
/// in WoW (see `keys`), and not before it.
const PASTE_PRESS_MS: i64 = 5000;
#[cfg(not(target_os = "macos"))]
const UNPROVEN_PASTE: &str = "T3 WoW pastes only right after you press Ctrl+V in its message box";
#[cfg(target_os = "macos")]
const UNPROVEN_PASTE: &str = "T3 WoW pastes only right after you press Cmd+V in its message box";
/// A refused paste when macOS may be hiding the keys (`keys::keys_hidden`): in the menu bar
/// app, and for a bare t3wow (allowed through the app it runs in).
#[cfg(target_os = "macos")]
const KEYS_HIDDEN: (&str, &str) = (
    "T3 WoW didn't see your Cmd+V: macOS may be hiding keys from it. Turn on T3 WoW in System Settings > Privacy & Security > Input Monitoring, then quit and reopen T3 WoW.",
    "t3wow didn't see your Cmd+V: macOS may be hiding keys from it. Turn on Terminal (or the app t3wow runs in) in System Settings > Privacy & Security > Input Monitoring, then restart it.",
);
#[cfg(not(target_os = "macos"))]
const KEYS_HIDDEN: &str = UNPROVEN_PASTE;
const NO_IMAGE: &str = "the clipboard holds no image";
/// Sends remembered until T3 shows their message.
const SENT_KEPT: usize = 16;
/// How far T3's clock may be behind this PC's (it may run on another machine) for a message
/// to still count as the one just sent.
const SENT_CLOCK_SKEW_MS: i64 = 120_000;

/// An image pasted in the game, waiting to go with the message it was pasted for.
struct Pasted {
    /// Its number, for the game to remove it by.
    n: u64,
    png: Vec<u8>,
    w: u32,
    h: u32,
    /// The image slot holding its small picture, and that picture's size (None: the game
    /// can't show pictures, or every slot is in use).
    thumb: Option<(u32, u32, u32)>,
    /// The same for its large picture, enlarged on a click and shown once sent.
    big: Option<(u32, u32, u32)>,
}

/// The large pictures of a message sent with images. T3's reads say nothing of a message's
/// attachments, so the companion ties them to the message itself when it first appears.
struct SentImages {
    env: String,
    thread: String,
    text: String,
    at: i64,
    slots: Vec<u32>,
}

/// A working thread's pending requests, as last read.
#[derive(Clone, Debug)]
struct Pending {
    /// The thread's `updatedAt` when read, and when that was.
    updated: String,
    read_at: i64,
    /// What it waits on: None, or the status the game shows ("waiting" for a question,
    /// "approval" for a permission prompt, which T3's pending list leaves out, CARD for a
    /// card only T3 can answer).
    status: Option<&'static str>,
    /// The card, when it waits on one.
    card: Option<Card>,
    /// The request behind the status, as T3's notifications key it: the newest question,
    /// or the card. None for an approval: T3's tools don't give its id.
    request: Option<String>,
}

/// A card only T3 can answer: "secret" or "link", and what it asks for.
#[derive(Clone, Debug, PartialEq)]
struct Card {
    kind: &'static str,
    label: String,
}

#[derive(Clone, Debug)]
struct PendingQuestion {
    key: String,
    labels: Vec<String>,
    multi: bool,
}

#[derive(Clone, Debug)]
struct PendingAsk {
    env: String,
    thread: String,
    token: String,
    request_id: String,
    questions: Vec<PendingQuestion>,
}

/// The machine part of a "{env}:{thread}" key.
fn env_of(key: &str) -> &str {
    key.split_once(':').map_or(key, |(env, _)| env)
}

fn with_args(base: Value, extra: &Map<String, Value>) -> Value {
    let mut base = base;
    if let Value::Object(map) = &mut base {
        for (k, v) in extra {
            map.insert(k.clone(), v.clone());
        }
    }
    base
}

/// A machine's sidebar threads, its settled count, and which lists have more.
type ThreadLists = Result<(Vec<Value>, u64, [bool; LIST_COUNT]), T3Error>;

/// A whole poll gets this long; past it, optional reads wait for the next one.
const POLL_DEADLINE_MS: i64 = 20_000;
/// How long what a machine offers (its providers and models) is kept before it's read again.
const CATALOG_TTL_MS: i64 = 300_000;
/// The permission modes a thread can start in, as T3 names them.
const RUNTIME_MODES: [&str; 4] = ["approval-required", "auto-accept-edits", "auto", "full-access"];
/// Command outcomes stay in the updates this long (the game takes each once), at most this many.
const OUTCOME_KEEP_MS: i64 = 60_000;
const OUTCOMES_KEPT: usize = 8;
/// A Stop whose run hasn't ended after this long is no longer waited on.
const STOP_PATIENCE_MS: i64 = 300_000;
/// How long one tick runs poll steps before it returns.
const TICK_BUDGET_MS: i64 = 300;
/// Mid-poll, publish only once the poll has run this long.
const PUBLISH_MIDPOLL_MS: i64 = 2000;
/// The open thread's messages are read at most this many pages a pass, and widened at
/// most this many passes.
const VIEW_PAGES: usize = 10;
const VIEW_PASSES: usize = 4;

/// A poll of T3, one step at a time. A step makes one request at most (a list step makes
/// one per machine, all at once), so between two requests the caller gets a turn: the
/// game's commands, the tray's controls and the capture never wait for more than one.
struct PollRun {
    started: i64,
    steps: VecDeque<Step>,
    /// Each machine's list replies so far, while the lists are read.
    lists: Vec<(String, Result<Vec<Value>, T3Error>)>,
    /// The machine whose lists are being taken in.
    listed: Option<Listed>,
    /// The open thread, while it's read.
    reading: Option<ViewRead>,
}

/// What one machine offers for the selectors.
#[derive(Default)]
struct Catalog {
    read_at: i64,
    /// `compact_providers`.
    providers: Vec<Value>,
    /// The permissions a new thread gets when none is picked.
    mode: Option<String>,
    /// What each provider's composer lists (`{ pv, c: compact_picks }`). Only this PC's T3.
    picks: Vec<Value>,
}

enum Step {
    /// The linked machines, then the server's own name (then each machine's projects).
    Machines,
    ServerName(Value),
    Projects(String),
    /// What a machine offers for the model and effort selectors.
    Catalog(String),
    /// A page of one of the three thread lists, every machine's at once (a list longer
    /// than T3's page takes a step per page).
    List(usize),
    /// One machine's lists, as read. Then, for it: each working thread's question check
    /// (and what it waits on), whether each thread with an open PR watches it, each launcher, the status changes, each active thread that
    /// left the lists, and keeping the lists.
    Machine(String, ThreadLists),
    Question(usize),
    QuestionKind(usize, i64),
    /// Which of its threads wait on a subagent still going.
    Delegates,
    Watch(usize),
    Launcher(usize),
    RunStart(usize),
    Activity(usize),
    Track,
    Unlisted(String, String),
    Keep,
    /// The open thread: its head, its messages a page at a time, its question.
    View,
    ViewConfig,
    ViewPage,
    ViewAsk,
    ViewAskRead(String),
    /// Its pictures: a page of its activity (looking for pages an agent showed), each
    /// picture to draw (a request at a time for a page's HTML), then the view with them.
    ViewScan,
    ViewCard(String),
    /// What you answered a question, from its position.
    ViewAnswer(u64),
    /// The whole list of files a turn changed, from its checkpoint item.
    ViewChanges(String),
    ViewDraw(usize),
    ViewPictures,
}

/// One machine's lists while they're taken in.
struct Listed {
    env: String,
    threads: Vec<Value>,
    settled_total: u64,
    more: [bool; LIST_COUNT],
    /// The working threads looked at: their pending counts stay, the machine's others go.
    seen: Vec<String>,
}

/// The open thread while it's read.
struct ViewRead {
    view: View,
    key: String,
    /// `view_gen` when the read began: a command that changed the view since drops it.
    gen: u64,
    thread: Value,
    /// Its model selection and modes (T3's read of a thread has no effort), when T3 gave them.
    config: Option<Value>,
    /// Its latest runs, each with the provider and model it ran on.
    runs: Value,
    /// When the head was read (a full read's time).
    began: i64,
    /// Reading on from this cache (same thread, safe to); None: a full read.
    cache: Option<ViewCache>,
    /// This pass's messages so far, where the next page starts, and pages read.
    messages: Vec<Value>,
    after: Option<u64>,
    pages: usize,
    /// A full read: the first item it reads, how many items back that is, passes done.
    start: usize,
    window: usize,
    passes: usize,
    /// The view for the game, once the messages are in, waiting on its question.
    built: Option<Value>,
    /// Each built item's position, and the images its messages embed from files.
    positions: Vec<u64>,
    embedded: Vec<Picture>,
    /// The pictures the view shows, and those drawn this poll.
    pictures: Vec<Picture>,
    drawing: Vec<Drawing>,
    scan_pages: usize,
    /// Something wasn't ready (pages left to look through, pictures left to draw): the
    /// next poll reads the view again.
    again: bool,
}

/// T3's sidebar is made of three lists per machine (open, snoozed, settled), read in turn.
const LIST_COUNT: usize = 3;
/// The lists as the game names them.
const LIST_NAMES: [&str; LIST_COUNT] = ["open", "snoozed", "settled"];
/// How many threads each list shows at first, and at most after Show more (each one
/// doubles it), which keeps an update's size reasonable.
const LIST_LIMITS: [usize; LIST_COUNT] = [80, 25, 25];
const LIST_CAPS: [usize; LIST_COUNT] = [320, 200, 200];
/// T3 lists at most this many threads a request; past it, a list is read a page at a time.
const LIST_PAGE: usize = 100;

/// The page a machine's list needs: its machine argument, where it starts, how many threads.
type ListPage = (Map<String, Value>, Option<u64>, usize);

/// A page of one of a machine's three lists: `limit` threads from `cursor` on.
fn fetch_list(client: &dyn T3, n: usize, cursor: Option<u64>, limit: usize, env_arg: &Map<String, Value>) -> Result<Value, T3Error> {
    let which = match n {
        0 => json!({ "settled": false, "snoozed": false }),
        1 => json!({ "settled": false, "snoozed": true }),
        _ => json!({ "settled": true }),
    };
    let mut base = with_args(json!({ "allProjects": true, "includeSubagents": false, "limit": limit }), which.as_object().unwrap());
    if let Some(cursor) = cursor {
        base["cursor"] = json!(cursor);
    }
    client.call("t3_thread_list", with_args(base, env_arg))
}

/// How many threads a list read so far holds.
fn listed_count(list: &Value) -> usize {
    list["threads"].as_array().map_or(0, Vec::len)
}

/// Where the next page of a list read so far starts, if it needs one to reach `limit`.
fn next_page(list: &Value, limit: usize) -> Option<u64> {
    list["nextCursor"].as_u64().filter(|_| listed_count(list) < limit)
}

/// A list has more threads than it holds: by its total, or, without one, by its cursor.
fn has_more(list: &Value) -> bool {
    match list["total"].as_u64() {
        Some(total) => total > listed_count(list) as u64,
        None => list["nextCursor"].is_u64(),
    }
}

/// A machine's sidebar threads from its three lists, its settled count, and which lists
/// have more.
fn combine_lists(lists: &[Value]) -> (Vec<Value>, u64, [bool; LIST_COUNT]) {
    let items = |n: usize| lists.get(n).and_then(|v| v["threads"].as_array()).cloned().unwrap_or_default();
    let mut threads: Vec<Value> = items(0).into_iter().filter(|t| t["snoozed"] != true).collect();
    threads.extend(items(1).into_iter().filter(|t| t["snoozed"] == true));
    threads.extend(items(2));
    threads.retain(|t| !is_subagent(t));
    let settled_listed = items(2).iter().filter(|t| on_sidebar(t)).count() as u64;
    let total = lists.get(2).and_then(|v| v["total"].as_u64()).unwrap_or(0);
    let more = [0, 1, 2].map(|n| lists.get(n).is_some_and(has_more));
    (threads, total.max(settled_listed), more)
}
impl Bridge {
    pub fn new(
        client: Arc<dyn T3>,
        slots: Box<dyn SlotWriter>,
        state_file: Option<PathBuf>,
        log: Box<dyn Fn(&str) + Send>,
        now: Box<dyn Fn() -> i64 + Send>,
    ) -> Bridge {
        let mut link = state_file
            .as_ref()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|text| serde_json::from_str::<LinkState>(&text).ok())
            .unwrap_or_default();
        // A picture whose file is gone or blank (a reinstall, a cleanup) is no picture: the
        // game would show a blank slot as a placeholder. An image you sent stays listed, so
        // its message can say it couldn't be loaded (place_pictures).
        link.images.retain(|n, slot| slot.key.starts_with(SENT_KEY) || slots.has_image(*n));
        let started_ms = now();
        Bridge {
            client,
            slots,
            state_file,
            log,
            now,
            opener: Box::new(crate::t3::open_browser),
            zone: Box::new(crate::snooze::system_offset),
            link,
            wow_seen: false,
            window_open: false,
            view: None,
            envs: vec![Env { id: String::new(), name: hostname(), ok: true }],
            local_env_id: None,
            threads: BTreeMap::new(),
            projects: BTreeMap::new(),
            new_worktrees: BTreeMap::new(),
            fresh_context: HashSet::new(),
            older_t3: None,
            catalogs: BTreeMap::new(),
            provider_caches: None,
            t3_settings: None,
            script_settings: None,
            snooze_presets: None,
            settled_totals: BTreeMap::new(),
            list_more: BTreeMap::new(),
            list_limits: LIST_LIMITS,
            view_data: None,
            view_key: None,
            view_cache: None,
            view_gen: 0,
            notes: Vec::new(),
            events: Vec::new(),
            event_seq: 0,
            last_status: HashMap::new(),
            shown_requests: HashMap::new(),
            unlisted_misses: HashMap::new(),
            pending_counts: HashMap::new(),
            watches: HashMap::new(),
            listed_envs: HashSet::new(),
            launchers: HashMap::new(),
            delegating: HashMap::new(),
            run_starts: HashMap::new(),
            activity: HashMap::new(),
            commands: VecDeque::new(),
            queued: HashMap::new(),
            last_publish_at: 0,
            last_published_hash: None,
            last_urgent_hash: None,
            held: BTreeMap::new(),
            // From the clock, so a restarted companion doesn't reuse the numbers the game
            // last saw. The game's JSON writes numbers as C ints: keep well under 2^31.
            update_id: (started_ms.div_euclid(1000).rem_euclid(1_000_000) * 1000) as u32,
            update_parts: BTreeMap::new(),
            poll_at: 0,
            poll_run: None,
            repoll: false,
            poll_failures: 0,
            list_errors: HashMap::new(),
            slow_at: 0,
            auth_error: None,
            sign_in_read_only: false,
            poll_error: None,
            polled_ok: false,
            pending_ask: None,
            outcomes: Vec::new(),
            stopping: HashMap::new(),
            signals_out: None,
            signal_failures: 0,
            slots_out_logged: None,
            write_error: None,
            write_failures: 0,
            trust: Trust::Full,
            game_key: None,
            unsigned: false,
            painter: None,
            images_ok: false,
            cards_ok: false,
            work_ok: false,
            details: Vec::new(),
            scan: None,
            picture_errors: HashMap::new(),
            clipboard: Box::new(clip::read_image),
            presses: Box::new(crate::keys::presses),
            arm_keys: Box::new(crate::keys::arm),
            keys_hidden: Box::new(crate::keys::keys_hidden),
            presses_used: 0,
            drafts: BTreeMap::new(),
            sent: Vec::new(),
            pasted_count: 0,
            started_ms,
            command_seen_at: 0,
            long_command: None,
        }
    }

    /// Draws pictures with `painter` from now on (None: none are drawn).
    pub fn set_painter(&mut self, painter: Option<Box<dyn Painter>>) {
        self.painter = painter;
    }

    /// Applies the tray's "Commands from the game" from the next command on.
    pub fn set_provider_caches(&mut self, dir: Option<PathBuf>) {
        self.provider_caches = dir;
    }

    pub fn set_t3_settings(&mut self, file: Option<PathBuf>) {
        self.t3_settings = file;
        self.script_settings = None;
        self.snooze_presets = None;
    }

    /// The key the addon signs its frames with: from then on, only signed frames may change T3.
    pub fn set_game_key(&mut self, key: Option<Vec<u8>>) {
        self.game_key = key;
    }

    pub fn set_trust(&mut self, trust: Trust) {
        if trust != self.trust {
            self.log(&format!("Commands from the game: {}", trust.label()));
            self.trust = trust;
        }
    }

    /// Whether the sign-in in use can only read (`Config::read_only`). The game shows it on
    /// its top line, where it shows a refused sign-in, since T3 would refuse every change.
    pub fn set_sign_in_read_only(&mut self, read_only: bool) {
        if read_only != self.sign_in_read_only {
            self.log(if read_only { "The T3 sign-in is Read only: T3 refuses every change from the game" } else { "The T3 sign-in can change threads" });
            self.sign_in_read_only = read_only;
        }
    }

    pub fn set_client(&mut self, client: Arc<dyn T3>) {
        self.client = client;
        self.auth_error = None;
        self.drop_poll();
    }

    /// The game's updates come from the T3 sign-in saved under `key` (`Config::server_key`):
    /// at every start, and whenever the companion moves to another T3. When that isn't the
    /// sign-in they came from before (state.json remembers it across restarts), everything
    /// from the previous one goes, here and in the game (see `new_login`). The same T3 on
    /// another port is the same sign-in: nothing goes.
    pub fn set_login(&mut self, key: &str) {
        if self.link.login_key.as_deref() == Some(key) && self.link.login.is_some() {
            return;
        }
        let why = match self.link.login_key.as_deref() {
            Some(_) => format!("T3 WoW moved to another T3 ({key})"),
            None => format!("T3 WoW uses the T3 sign-in for {key}"),
        };
        self.link.login_key = Some(key.to_string());
        self.drop_poll();
        self.new_login(&why);
    }

    /// The tray's Sign out: what was read with that sign-in goes, here and in the game.
    pub fn sign_out(&mut self) {
        self.drop_poll();
        self.new_login("Signed out of T3");
    }

    /// A poll under way reads the previous T3 (or sign-in): drop it and start afresh.
    fn drop_poll(&mut self) {
        self.poll_run = None;
        self.poll_failures = 0;
        self.poll_at = 0;
        self.slow_at = 0;
    }

    /// A new T3 sign-in: forgets what the previous one showed, gives the game's updates a
    /// new stamp (the game drops its own copy of the previous one's threads, open thread,
    /// notifications and waiting commands when it sees it), blanks the slot files that
    /// still hold the previous one's threads, and sends the game the new state.
    fn new_login(&mut self, why: &str) {
        self.forget_server();
        self.envs = vec![Env { id: String::new(), name: hostname(), ok: true }];
        self.local_env_id = None;
        self.events.clear();
        self.notes.clear();
        self.outcomes.clear();
        self.drafts.clear();
        self.sent.clear();
        self.list_errors.clear();
        self.polled_ok = false;
        self.auth_error = None;
        self.poll_error = None;
        self.view_gen += 1;
        self.link.login = Some(new_login_stamp());
        self.slots.clear_slots();
        self.republish();
        self.save_state();
        self.log(&format!("{why}: the game drops the previous sign-in's threads"));
    }

    /// Drops what was read from the T3 (its machines' threads, projects, the open thread).
    fn forget_server(&mut self) {
        self.view = None;
        self.view_cache = None;
        self.view_data = None;
        self.view_key = None;
        self.pending_ask = None;
        self.threads.clear();
        self.projects.clear();
        self.run_starts.clear();
        self.activity.clear();
        self.new_worktrees.clear();
        self.fresh_context.clear();
        self.older_t3 = None;
        self.catalogs.clear();
        self.settled_totals.clear();
        self.list_more.clear();
        self.last_status.clear();
        self.shown_requests.clear();
        self.unlisted_misses.clear();
        self.pending_counts.clear();
        self.watches.clear();
        self.launchers.clear();
        self.delegating.clear();
        self.listed_envs.clear();
        self.stopping.clear();
    }

    fn now_ms(&self) -> i64 {
        (self.now)()
    }

    fn now_seconds(&self) -> i64 {
        self.now_ms().div_euclid(1000)
    }

    /// The signal file the game watches at `now_s`, by the same arithmetic as Link.lua.
    fn watched_signal(&self, now_s: i64) -> u32 {
        let start = if self.link.signal_time != 0 { self.link.signal_time } else { now_s };
        self.link.signal + (now_s - start).max(0).div_euclid(SIGNAL_PERIOD) as u32
    }

    fn save_state(&self) {
        if let Some(file) = &self.state_file {
            let _ = std::fs::write(file, serde_json::to_string(&self.link).unwrap_or_default());
        }
    }

    /// A thread New Thread started from the game: yours, though T3 records it as an agent's.
    fn remember_started(&mut self, id: &str) {
        let started = &mut self.link.started_threads;
        if !started.iter().any(|known| known == id) {
            started.push(id.to_string());
            let excess = started.len().saturating_sub(STARTED_KEPT);
            started.drain(..excess);
            self.save_state();
        }
    }

    /// A message sent in a fresh session, by the id T3's send answered with.
    fn remember_fresh(&mut self, sent: &Value) {
        let Some(message) = sent["messageId"].as_str().filter(|m| !m.is_empty()) else { return };
        let fresh = &mut self.link.fresh_messages;
        if !fresh.iter().any(|known| known == message) {
            fresh.push(message.to_string());
            let excess = fresh.len().saturating_sub(FRESH_KEPT);
            fresh.drain(..excess);
            self.save_state();
        }
    }

    fn log(&self, message: &str) {
        (self.log)(message);
    }

    fn note(&mut self, text: &str) {
        let at = self.now_seconds();
        self.notes.push(json!({ "at": at, "x": plain(text, 300) }));
        let excess = self.notes.len().saturating_sub(4);
        self.notes.drain(..excess);
    }

    pub fn status(&self) -> BridgeStatus {
        BridgeStatus {
            t3_ok: self.polled_ok && self.auth_error.is_none() && self.poll_error.is_none(),
            t3_problem: self.auth_error.clone().or_else(|| self.poll_error.clone()),
            auth_needed: self.auth_error.is_some(),
            sign_in_read_only: self.sign_in_read_only,
            wow_running: self.wow_seen && self.link.wow_key.as_deref().is_some_and(|k| !k.is_empty()),
            in_game: self.link.ui_session.is_some(),
            signals: self.link.polling == Some(false) && self.signals_out.is_none(),
            machines: self.envs.iter().filter(|e| e.ok).count(),
            slot: self.link.slot,
            signals_out: self.signals_out.is_some(),
            signals_lost: self.signals_out.as_deref() == Some(signals_lost()),
            write_problem: self.write_error.clone().filter(|_| self.write_failures >= WRITE_FAILURES_SHOWN),
            older_t3: self.older_t3.clone(),
        }
    }

    // ---- input from the capture ----

    pub fn on_capture(&mut self, message: CaptureMessage) {
        match message {
            CaptureMessage::Wow(key) => {
                self.wow_seen = true;
                let key = key.unwrap_or_default();
                if Some(&key) != self.link.wow_key.as_ref() {
                    self.log(&if key.is_empty() { "WoW not running".to_string() } else { format!("WoW running ({key})") });
                    // A new client process hasn't loaded any signal font: all start blank again.
                    self.slots.clear_signals();
                    self.slots.flip_probe(false);
                    self.signals_out = None;
                    self.signal_failures = 0;
                    self.link.signal = 1;
                    self.link.signal_time = self.now_seconds();
                    self.link.wow_key = Some(key);
                    self.link.ui_session = None;
                    self.new_game_images();
                    self.save_state();
                }
            }
            CaptureMessage::Strip(cells, at) => {
                if let Some(frame) = decode_cells(&cells) {
                    if let Ok(header) = serde_json::from_str::<Value>(&frame.payload) {
                        let signed = self.game_key.as_ref().is_some_and(|key| frame.signed_with(key));
                        self.on_frame(&header, frame.part, signed, at);
                    }
                }
            }
        }
    }

    /// A new game process has loaded no picture: every image slot can be written again. The
    /// most recently shown keep theirs (a thread opened again finds them); the rest give
    /// their disk space back.
    fn new_game_images(&mut self) {
        self.link.images_shown.clear();
        // Images pasted in a game that quit: their pictures' slots can go to others.
        self.drafts.clear();
        self.sent.clear();
        let mut by_use: Vec<(i64, u32)> = self.link.images.iter().map(|(n, slot)| (slot.used, *n)).collect();
        by_use.sort_unstable_by(|a, b| b.cmp(a));
        for (_, n) in by_use.into_iter().skip(IMAGES_KEPT) {
            self.link.images.remove(&n);
            self.slots.blank_image(n);
        }
    }

    /// Keeps one part of a long command (`pt` says which). When that was the last one
    /// missing, returns the whole command.
    fn take_part(&mut self, k: &str, u: &str, pt: &Value, part: Vec<u8>, signed: bool, seen: u64) -> Option<(Value, bool)> {
        let (s, index, count) = (pt["s"].as_u64()?, pt["i"].as_u64()?, pt["n"].as_u64()?);
        if s <= seen || count > MAX_PARTS || index == 0 || index > count {
            return None;
        }
        let (from, zlib) = ((k.to_string(), u.to_string(), s), pt["z"] == 1);
        if self.long_command.as_ref().is_none_or(|long| long.from != from || long.zlib != zlib || long.parts.len() as u64 != count) {
            self.long_command = Some(LongCommand { from, zlib, parts: vec![None; count as usize], signed: true });
        }
        let long = self.long_command.as_mut()?;
        long.parts[index as usize - 1] = Some(part);
        long.signed &= signed;
        if long.parts.iter().any(Option::is_none) {
            return None;
        }
        let long = self.long_command.take()?;
        let body: Vec<u8> = long.parts.into_iter().flatten().flatten().collect();
        Some((read_long_command(&body, zlib, s), long.signed))
    }

    /// A frame from the game's strip, read at `at` (epoch ms), with the raw bytes of a part
    /// of a long command if it has one, and whether it was signed with the addon's key.
    pub fn on_frame(&mut self, frame: &Value, part: Option<Vec<u8>>, signed: bool, at: i64) {
        let (Some(k), Some(u)) = (frame["k"].as_str(), frame["u"].as_str()) else { return };
        // An addon that can show pictures says so in every frame.
        let images_ok = frame["im"] == 1;
        if images_ok != self.images_ok {
            self.images_ok = images_ok;
            if images_ok && self.painter.is_some() {
                self.log("The game can show pictures: pages and images in threads go to it as pictures");
            }
            self.view_changed();
        }
        // An addon that draws delegated tasks as cards says so in every frame.
        let cards_ok = frame["dc"] == 1;
        if cards_ok != self.cards_ok {
            self.cards_ok = cards_ok;
            self.view_changed();
        }
        // And one that draws what the agent did as rows.
        let work_ok = frame["wk"] == 1;
        if work_ok != self.work_ok {
            self.work_ok = work_ok;
            self.view_changed();
        }
        let l = frame["l"].as_u64().unwrap_or(0) as u32;
        // Until the game watches signals (it learns the starting file from its first update,
        // and some clients can't use them) it loads slots on a timer and reports where it is.
        let polling = frame["p"] == 1;
        if Some(u) != self.link.ui_session.as_deref() || Some(k) != self.link.client_key.as_deref() {
            // New UI session (login or /reload): every slot can load again. Signal files raised
            // earlier in this game process stay real, so the new wait starts past all of them.
            let now_s = self.now_seconds();
            self.link.ui_session = Some(u.to_string());
            self.link.client_key = Some(k.to_string());
            self.link.slot = l.max(1);
            self.link.signal = self.watched_signal(now_s) + 2;
            self.link.signal_time = now_s;
            self.last_published_hash = None;
            self.last_urgent_hash = None;
            self.held.clear();
            self.link.polling = None;
            // The game's command numbers live in its saved variables, which a crash rolls
            // back: start counting from what this session sends. A command run twice this
            // way reaches T3 with the same clientRequestId, which T3 ignores.
            if let Some(lowest) = frame["q"].as_array().into_iter().flatten().filter_map(|c| c["s"].as_u64()).chain(frame["pt"]["s"].as_u64()).min() {
                self.link.acked.insert(k.to_string(), lowest.saturating_sub(1));
            }
            self.queued.remove(k);
            self.outcomes.clear();
            // Show more lasts for the UI session it was asked in.
            self.list_limits = LIST_LIMITS;
            self.log(&format!("Game UI connected (session {u}, next slot {})", self.link.slot));
        } else {
            if l > 0 {
                // The game draws a frame with its current slot count, so it is the authority on
                // where it loads next (it may have loaded extra slots on its own).
                self.link.slot = l;
            }
            // The update the game is in step with: it holds that update's parts. Only the
            // latest counts, as a part in a later update may not have reached it. 0: a part
            // an update left out wasn't the one the game had, so send them all again.
            match frame["g"].as_u64() {
                Some(0) if !self.held.is_empty() => {
                    self.held.clear();
                    // Even if nothing else changed: what it holds needs replacing now.
                    self.last_published_hash = None;
                    self.last_urgent_hash = None;
                }
                Some(g) if g == u64::from(self.update_id) => self.held = self.update_parts.clone(),
                _ => {}
            }
        }
        if Some(polling) != self.link.polling {
            if !polling {
                self.log(&format!("Game watching signals (file {}): updates arrive in about a second", frame["n"]));
            } else if self.link.polling == Some(false) {
                self.log("Game stopped watching signals: it checks for updates on a timer");
            }
            self.link.polling = Some(polling);
        }
        if signed == self.unsigned {
            self.unsigned = !signed;
            self.log(if self.unsigned {
                "The game's frames aren't signed with the addon's key: commands from it can only read until WoW restarts"
            } else {
                "The game's frames are signed with the addon's key"
            });
        }
        let seen = self.link.acked.get(k).copied().unwrap_or(0).max(self.queued.get(k).copied().unwrap_or(0));
        let long = part.and_then(|part| self.take_part(k, u, &frame["pt"], part, signed, seen));
        let queued = frame["q"].as_array().into_iter().flatten().map(|command| (command, signed));
        for (command, signed) in queued.chain(long.iter().map(|(command, signed)| (command, *signed))) {
            let s = command["s"].as_u64().unwrap_or(0);
            if s <= seen {
                continue;
            }
            let entry = self.queued.entry(k.to_string()).or_insert(0);
            *entry = (*entry).max(s);
            self.commands.push_back((k.to_string(), u.to_string(), frame["lid"].as_str().map(str::to_string), command.clone(), at, signed));
        }
        self.save_state();
    }

    // ---- commands from the game ----

    fn env_arg(&self, env: &str) -> Map<String, Value> {
        let mut map = Map::new();
        if !env.is_empty() && Some(env) != self.local_env_id.as_deref() {
            map.insert("environmentId".into(), json!(env));
        }
        map
    }

    /// Runs one command from the game. Returns its outcome for the game, if it has one to
    /// know: a failure (so the window can put the text back), or the thread a "new" started.
    fn run(&mut self, client_key: &str, login: Option<&str>, command: Value, signed: bool) -> Option<Value> {
        // No machine named: the server's own.
        let env = match str_of(&command["e"]) {
            "" => self.local_env_id.clone().unwrap_or_default(),
            env => env.to_string(),
        };
        let id = str_of(&command["id"]).to_string();
        let message = str_of(&command["m"]).to_string();
        let kind = str_of(&command["t"]).to_string();
        if kind == GARBLED {
            self.log("A long message from the game couldn't be read");
            return Some(json!({ "s": command["s"], "ok": false, "x": GARBLED_REASON }));
        }
        // The command's number plus a hash of it: the same command sent again (after a
        // /reload) is ignored by T3, while a new one that reuses a number after a crash isn't.
        let request_id = format!("t3wow:{client_key}:{}:{:08x}", command["s"], hash_of(&command) as u32);
        // Drawn while the game still showed another T3 sign-in's threads: its thread and
        // machine ids (or the task it starts) were meant for that one, not this.
        if login.is_some_and(|l| Some(l) != self.link.login.as_deref()) && !LOGIN_FREE.contains(&kind.as_str()) {
            self.log(&format!("{kind} from the game {OTHER_LOGIN}"));
            return Some(json!({ "s": command["s"], "ok": false, "x": OTHER_LOGIN }));
        }
        // Any addon can draw commands, but only T3 WoW can sign them: an unsigned one may
        // only read.
        if !signed && !Trust::ReadOnly.allows(&kind) {
            self.log(&format!("{kind} from the game {UNSIGNED_REASON}"));
            return Some(json!({ "s": command["s"], "ok": false, "x": UNSIGNED_REASON }));
        }
        // Then the tray setting decides what the game may do.
        if !self.trust.allows(&kind) {
            let reason = format!("blocked by {} ({})", crate::app::shown(TRUST_SETTING), self.trust.label());
            self.log(&format!("{kind} from the game {reason}"));
            self.note(&format!("{kind}: {reason}"));
            return Some(json!({ "s": command["s"], "ok": false, "x": reason }));
        }
        let env_arg = self.env_arg(&env);
        // The images pasted for this message, if it is one.
        let draft = if kind == "new" { "new".to_string() } else { format!("{}:{id}", str_of(&command["e"])) };
        let with_images = matches!(kind.as_str(), "send" | "new") && self.drafts.get(&draft).is_some_and(|d| !d.is_empty());
        let mut started: Option<String> = None;
        let mut stop_state: Option<&str> = None;
        // A command the game hears the outcome of even when it went through.
        let mut acked = false;
        let result: Result<(), T3Error> = (|| {
            match kind.as_str() {
                "hello" | "ui" => {
                    self.window_open = command["o"] == true;
                    // Ctrl+V is looked for only while T3 WoW's window is open (see `keys`). An
                    // older addon also says whether you're typing (`f`): no longer used, since
                    // that news came too late for a Ctrl+V right after the click.
                    (self.arm_keys)(self.window_open);
                }
                "open" => {
                    self.view = (!id.is_empty()).then(|| View { env: env.clone(), id: id.clone(), depth: 1 });
                    self.view_changed();
                }
                "more" => {
                    if let Some(view) = &mut self.view {
                        view.depth += 1;
                    }
                    self.view_changed();
                }
                "detail" => {
                    if let (Some(first), Some(last), true) = (command["p"].as_u64(), command["l"].as_u64(), self.view.as_ref().is_some_and(|v| v.env == env && v.id == id)) {
                        self.open_detail(&env, &id, first, last)?;
                    }
                }
                "send" if !id.is_empty() && (!message.is_empty() || with_images) => {
                    // T3's /clear: this message starts a fresh provider session in the thread.
                    let fresh = command["fc"] == true;
                    if fresh && !self.fresh_context_on(&env) {
                        return Err(T3Error::Other("this T3 can't start a fresh session: update it to use /clear".into()));
                    }
                    if fresh && with_images {
                        return Err(T3Error::Other("T3 can't start a fresh session with pasted images: send them after it starts".into()));
                    }
                    if with_images {
                        // T3's send with attachments has no mode or retry key: it starts an
                        // idle thread, and steers or queues on a busy one, as a send does.
                        let attachments = self.upload_draft(&draft)?;
                        let mut args = json!({ "threadId": id, "attachments": attachments });
                        if !message.is_empty() {
                            args["message"] = json!(message);
                        }
                        if let Err(error) = self.client.call("t3_thread_send_attachments", with_args(args, &env_arg)) {
                            self.discard_uploads(&attachments);
                            return Err(error);
                        }
                        self.remember_sent(&draft, &env, &id, &message);
                    } else {
                        let mut args = json!({ "threadId": id, "message": message, "mode": "auto", "clientRequestId": request_id });
                        if fresh {
                            args["freshContext"] = json!(true);
                        }
                        let sent = self.client.call("t3_thread_send", with_args(args, &env_arg))?;
                        if fresh {
                            self.remember_fresh(&sent);
                        }
                    }
                    self.note(if fresh { "Sent in a fresh session." } else { "Sent." });
                    let depth = self.view.as_ref().filter(|v| v.id == id).map(|v| v.depth).unwrap_or(1);
                    self.view = Some(View { env: env.clone(), id: id.clone(), depth });
                    self.view_changed();
                }
                "new" if !message.is_empty() || with_images => {
                    let mut title: String = message.split('\n').next().unwrap_or("").chars().take(60).collect();
                    if title.trim().is_empty() {
                        title = "Pasted image".into();
                    }
                    let mut args = json!({ "title": title, "clientRequestId": request_id });
                    if !message.is_empty() {
                        args["message"] = json!(message);
                    }
                    match command["p"].as_str() {
                        Some(project) => args["projectId"] = json!(project),
                        None => args["scratch"] = json!(true),
                    }
                    // A new worktree branches from what the project's checkout has checked
                    // out; without one, the thread runs in that checkout (T3's default).
                    if command["p"].is_string() && command["w"] == true {
                        args["workspaceStrategy"] = json!({ "type": "worktree", "baseRef": "HEAD" });
                    }
                    if let Some(selection) = self.model_selection(&env, &command, Vec::new())? {
                        args["modelSelection"] = selection;
                    }
                    // Starting without a picked mode would run at T3's default, usually the
                    // most access the sign-in has: a mode it can't name doesn't get that.
                    if let Some(mode) = command["rm"].as_str() {
                        if !RUNTIME_MODES.contains(&mode) {
                            return Err(T3Error::Other(format!("unknown permission mode \"{}\"", plain(mode, 20))));
                        }
                        args["runtimeMode"] = json!(mode);
                    }
                    let attachments = if with_images { self.upload_draft(&draft)? } else { Vec::new() };
                    if !attachments.is_empty() {
                        args["attachments"] = json!(attachments);
                    }
                    let launched = match self.client.call("t3_thread_launch", with_args(args, &env_arg)) {
                        Ok(launched) => launched,
                        Err(error) => {
                            self.discard_uploads(&attachments);
                            return Err(error);
                        }
                    };
                    let new_id = launched["threadId"].as_str().or(launched["thread"]["threadId"].as_str()).map(str::to_string);
                    self.remember_sent(&draft, &env, new_id.as_deref().unwrap_or(""), &message);
                    self.note(&format!("Started \"{title}\"."));
                    if let Some(new_id) = new_id {
                        self.remember_started(&new_id);
                        self.view = Some(View { env: env.clone(), id: new_id.clone(), depth: 1 });
                        started = Some(new_id);
                    }
                    self.view_changed();
                }
                // The model and effort of a thread that exists. T3 replaces the options it's
                // given, so the ones the game doesn't show (fast mode, say) are sent back. Its
                // permissions can't change from here: T3's tools keep a thread's modes.
                "model" if !id.is_empty() && !str_of(&command["md"]).is_empty() => {
                    let current = self.client.call("t3_thread_configure", with_args(json!({ "threadId": id }), &env_arg))?;
                    let selection = &current["modelSelection"];
                    let same = selection["model"] == command["md"] && selection["instanceId"] == command["pv"];
                    let kept = if same { selection["options"].as_array().cloned().unwrap_or_default() } else { Vec::new() };
                    let chosen = self.model_selection(&env, &command, kept)?;
                    self.client.call("t3_thread_configure", with_args(json!({ "threadId": id, "modelSelection": chosen }), &env_arg))?;
                    self.note("Model changed.");
                    self.view_changed();
                }
                "settle" | "unsettle" if !id.is_empty() => {
                    self.client.call("t3_thread_organize", with_args(json!({ "action": kind, "threadId": id }), &env_arg))?;
                    self.note(if kind == "settle" { "Settled." } else { "Moved back to open." });
                }
                // A time typed in the game ("at": "fri 3pm"), or one of the presets it was
                // offered ("k": "at:today:18:00"), read on this PC's clock. The game hears
                // either way, so a typed time it couldn't read stays in its box.
                "snooze" if !id.is_empty() && (command["at"].is_string() || command["k"].is_string()) => {
                    let now = self.now_seconds();
                    let until = match command["at"].as_str() {
                        Some(typed) => crate::snooze::resolve_typed(typed, now, &*self.zone, MAX_SNOOZE_MINUTES).map_err(|e| T3Error::Other(e.message(typed)))?,
                        None => crate::snooze::preset_from_key(str_of(&command["k"]))
                            .and_then(|preset| crate::snooze::preset_choice(&preset, now, &*self.zone))
                            .map(|choice| choice.until)
                            .filter(|until| until - now <= MAX_SNOOZE_MINUTES * 60)
                            .ok_or_else(|| T3Error::Other("that snooze time has passed: pick another".into()))?,
                    };
                    self.client.call("t3_thread_organize", with_args(json!({ "action": "snooze", "threadId": id, "snoozedUntil": iso_from_seconds(until) }), &env_arg))?;
                    self.note(&format!("Snoozed until {}.", crate::snooze::describe(until, now, &*self.zone)));
                    acked = true;
                }
                // An older addon says for how long; T3 takes the time it ends.
                "snooze" if !id.is_empty() && command["mn"].as_i64().is_some_and(|m| (1..=MAX_SNOOZE_MINUTES).contains(&m)) => {
                    let until = iso_from_seconds(self.now_seconds() + command["mn"].as_i64().unwrap_or(0) * 60);
                    self.client.call("t3_thread_organize", with_args(json!({ "action": "snooze", "threadId": id, "snoozedUntil": until }), &env_arg))?;
                    self.note("Snoozed.");
                }
                "unsnooze" if !id.is_empty() => {
                    self.client.call("t3_thread_organize", with_args(json!({ "action": "unsnooze", "threadId": id }), &env_arg))?;
                    self.note("Moved back to open.");
                }
                // T3 says whether it stopped a run ("interrupt_requested") or there was none
                // ("no_active_run", or how the last one ended; a T3 that says neither counts as
                // stopping). The game hears which, so its Stop doesn't wait on a run that already
                // ended; a refusal is an error, like any command's.
                "stop" if !id.is_empty() => {
                    let stopped = self.client.call("t3_thread_interrupt", with_args(json!({ "threadId": id, "clientRequestId": request_id }), &env_arg))?;
                    let key = format!("{env}:{id}");
                    if matches!(str_of(&stopped["status"]), "no_active_run" | "completed" | "failed" | "cancelled" | "interrupted" | "rolled_back") {
                        self.stopping.remove(&key);
                        self.note("Nothing to stop: the run had already ended.");
                        stop_state = Some("idle");
                    } else {
                        let now = self.now_ms();
                        self.stopping.retain(|_, at| now - *at < STOP_PATIENCE_MS);
                        self.stopping.insert(key, now);
                        self.note("Stopping.");
                        stop_state = Some("stopping");
                    }
                }
                // Brings T3 to the front on that thread (T3's own t3code:// link), for what only
                // T3 can do, like approving a command.
                "t3" => {
                    let machine = if env.is_empty() { self.local_env_id.clone().unwrap_or_default() } else { env.clone() };
                    let link = if id.is_empty() || machine.is_empty() {
                        "t3code://open".to_string()
                    } else {
                        format!("t3code://threads/{}/{}", crate::t3::percent_encode(&machine), crate::t3::percent_encode(&id))
                    };
                    (self.opener)(&link);
                    self.note("Opened in T3.");
                }
                // Any addon can draw commands: only what T3 has linked to this thread opens.
                "pr" => {
                    let url = str_of(&command["u"]);
                    let linked = self.threads.get(&env).is_some_and(|list| {
                        list.iter().filter(|t| t["threadId"] == id.as_str()).any(|t| numbered_pull_requests(t).iter().any(|pr| pr["url"] == url))
                    });
                    if !linked || !is_web_url(url) {
                        return Err(T3Error::Other("that pull request isn't linked to this thread".into()));
                    }
                    (self.opener)(url);
                }
                "probe" => {
                    let changed = self.slots.flip_probe(true);
                    self.log(&if changed.is_empty() { "probe: no probe files; reinstall the addon".into() } else { format!("probe: made {} files real", changed.join(", ")) });
                }
                "answer" if !id.is_empty() => {
                    let ask = self
                        .pending_ask
                        .clone()
                        .filter(|a| a.thread == id && a.env == env && a.token == str_of(&command["k"]))
                        .ok_or_else(|| T3Error::Other("that question was already answered or has changed".into()))?;
                    let answers = Self::answers_for(&ask, &command["a"]);
                    if answers.is_empty() {
                        return Err(T3Error::Other("pick an answer first".into()));
                    }
                    let args = json!({ "action": "respond", "threadId": id, "requestId": ask.request_id, "answers": answers });
                    self.client.call("t3_pending_request", with_args(args, &env_arg))?;
                    self.pending_counts.remove(&format!("{env}:{id}"));
                    self.note("Answered.");
                    self.pending_ask = None;
                    self.view_changed();
                }
                // An image pasted in the message box, read from the clipboard here: only right
                // after a real Ctrl+V in WoW, which no addon can fake.
                "paste" if !str_of(&command["d"]).is_empty() => {
                    let draft = str_of(&command["d"]).to_string();
                    self.check_paste_press()?;
                    if self.drafts.get(&draft).map_or(0, Vec::len) >= MAX_PASTED {
                        return Err(T3Error::Other(format!("a message takes {MAX_PASTED} images at most")));
                    }
                    let clipped = (self.clipboard)().map_err(T3Error::Other)?.ok_or_else(|| T3Error::Other(NO_IMAGE.into()))?;
                    self.stage(draft, clipped);
                    self.note("Image pasted.");
                    acked = true;
                }
                "unpaste" => {
                    let n = command["i"].as_u64().unwrap_or(0);
                    if let Some(images) = self.drafts.get_mut(str_of(&command["d"])) {
                        images.retain(|image| image.n != n);
                    }
                    self.drafts.retain(|_, images| !images.is_empty());
                    self.note("Image removed.");
                }
                // T3's tools can't run a saved script, so the thread's agent is asked to. Any
                // addon can draw commands: the game names a script of the open thread's
                // project, and what it runs comes from T3.
                "script" if !id.is_empty() => {
                    let project = self.view_data.as_ref().filter(|v| v["id"] == id.as_str() && v["e"] == env.as_str()).map(|v| str_of(&v["pid"]).to_string()).unwrap_or_default();
                    let script = self
                        .project_scripts(&env, &project)
                        .unwrap_or(&[])
                        .iter()
                        .find(|s| s["id"] == command["sc"])
                        .ok_or_else(|| T3Error::Other("that script isn't saved in this thread's project any more".into()))?;
                    let name = str_of(&script["name"]).to_string();
                    let message = script_request(&name, str_of(&script["command"]));
                    let args = json!({ "threadId": id, "message": message, "mode": "auto", "clientRequestId": request_id });
                    self.client.call("t3_thread_send", with_args(args, &env_arg))?;
                    self.note(&format!("Asked the agent to run \"{}\".", plain(&name, 40)));
                    self.view_changed();
                }
                "refresh" => self.view_changed(),
                // Show more: the next poll lists more of that group, up to its cap.
                "list" => {
                    if let Some(n) = LIST_NAMES.iter().position(|g| *g == str_of(&command["g"])) {
                        self.list_limits[n] = (self.list_limits[n] * 2).min(LIST_CAPS[n]);
                    }
                }
                _ => return Ok(()),
            }
            Ok(())
        })();
        // Look at T3 again now (or as soon as the poll under way ends).
        self.poll_at = 0;
        self.repoll = self.poll_run.is_some();
        match result {
            Err(error) => {
                self.log(&format!("{kind} failed: {error}"));
                self.note(&format!("{kind} failed: {error}"));
                Some(json!({ "s": command["s"], "ok": false, "x": plain(&error.to_string(), 200) }))
            }
            Ok(()) => started
                .map(|id| json!({ "s": command["s"], "ok": true, "env": env, "id": id }))
                .or_else(|| stop_state.map(|st| json!({ "s": command["s"], "ok": true, "st": st })))
                .or_else(|| acked.then(|| json!({ "s": command["s"], "ok": true }))),
        }
    }

    // ---- pasted images ----

    /// Lets a paste through only if a Ctrl+V in WoW came just before the frame that brought
    /// it, and no other paste used that press.
    fn check_paste_press(&mut self) -> Result<(), T3Error> {
        let (count, at) = (self.presses)();
        let seen = self.command_seen_at;
        if count <= self.presses_used || at > seen || seen - at > PASTE_PRESS_MS {
            // On a Mac without Input Monitoring the keys may not come through at all: then
            // say how to turn it on (macOS has just been asked to list T3 WoW there).
            if (self.keys_hidden)() {
                self.log("paste from the game without a Cmd+V seen in WoW, and macOS may be hiding the keys (no Input Monitoring): refused");
                return Err(T3Error::Other(crate::app::shown(KEYS_HIDDEN).into()));
            }
            self.log("paste from the game without a Ctrl+V in WoW just before it: refused");
            return Err(T3Error::Other(UNPROVEN_PASTE.into()));
        }
        self.presses_used = count;
        Ok(())
    }

    /// Puts a picture of a pasted image in an image slot and names it to the game process:
    /// the slot and the picture's size, None if it couldn't be stored.
    fn show_pasted(&mut self, key: &str, bitmap: &Bitmap) -> Option<(u32, u32, u32)> {
        if let Err(error) = self.store_picture(key, bitmap) {
            self.log(&format!("couldn't show the pasted image in the game: {error}"));
            return None;
        }
        let now = self.now_ms();
        let (slot, image) = self.link.images.iter_mut().find(|(_, image)| image.key == key)?;
        image.used = now;
        let slot = *slot;
        if !self.link.images_shown.contains(&slot) {
            self.link.images_shown.push(slot);
        }
        self.save_state();
        Some((slot, bitmap.width, bitmap.height))
    }

    /// Keeps a pasted image for `draft`'s message, with its pictures in image slots for the
    /// game: a small one above the box, and a large one for a click and for after it's sent.
    fn stage(&mut self, draft: String, clipped: Clipped) {
        self.pasted_count += 1;
        let n = self.pasted_count;
        let (mut thumb, mut big) = (None, None);
        if self.images_ok {
            let key = format!("paste:{}:{n}", self.started_ms);
            thumb = self.show_pasted(&key, &clip::thumbnail(&clipped.bitmap));
            big = self.show_pasted(&format!("{key}:big"), &clip::preview(&clipped.bitmap));
        }
        let (w, h) = (clipped.bitmap.width, clipped.bitmap.height);
        self.log(&format!("Image pasted from the clipboard ({w}x{h}, {} KB)", clipped.png.len() / 1024));
        self.drafts.entry(draft).or_default().push(Pasted { n, png: clipped.png, w, h, thumb, big });
    }

    /// `draft`'s images went out with `text`: they leave the box, and their large pictures
    /// wait for the message to appear in the thread.
    fn remember_sent(&mut self, draft: &str, env: &str, thread: &str, text: &str) {
        let images = self.drafts.remove(draft).unwrap_or_default();
        let slots: Vec<u32> = images.iter().filter_map(|image| image.big.map(|(slot, _, _)| slot)).collect();
        if slots.is_empty() || thread.is_empty() {
            return;
        }
        let at = self.now_ms();
        self.sent.push(SentImages { env: env.to_string(), thread: thread.to_string(), text: text.to_string(), at, slots });
        if self.sent.len() > SENT_KEPT {
            self.sent.remove(0);
        }
    }

    /// The image slot keys of the pictures sent with this user message, in order. The first
    /// time a message appears that matches a send (the same words, not older than the send;
    /// the newest send first), its pictures are filed under T3's id for the message, so they
    /// stay with it from then on.
    fn sent_pictures(&mut self, env: &str, id: &str, item: &Value) -> Vec<String> {
        let message = str_of(&item["messageId"]);
        if message.is_empty() {
            return Vec::new();
        }
        let prefix = format!("{SENT_KEY}{env}:{id}:{message}:");
        if !self.link.images.values().any(|slot| slot.key.starts_with(&prefix)) {
            let said = str_of(&item["text"]).trim();
            let cut_short = item["textTruncated"] == true;
            let at = iso_seconds(str_of(&item["updatedAt"])) * 1000;
            let found = self.sent.iter().rposition(|sent| {
                let same = if cut_short { sent.text.trim().starts_with(said) } else { sent.text.trim() == said };
                sent.env == env && sent.thread == id && same && at >= sent.at - SENT_CLOCK_SKEW_MS
            });
            if let Some(found) = found {
                let sent = self.sent.remove(found);
                for (i, slot) in sent.slots.iter().enumerate() {
                    if let Some(image) = self.link.images.get_mut(slot) {
                        image.key = format!("{prefix}{i}");
                    }
                }
                self.save_state();
            }
        }
        let mut kept: Vec<(u32, String)> = self
            .link
            .images
            .values()
            .filter_map(|slot| Some((slot.key.strip_prefix(&prefix)?.parse().ok()?, slot.key.clone())))
            .collect();
        kept.sort();
        kept.into_iter().map(|(_, key)| key).collect()
    }

    /// Uploads `draft`'s images to T3, as attachments for a message.
    fn upload_draft(&self, draft: &str) -> Result<Vec<Value>, T3Error> {
        let mut attachments: Vec<Value> = Vec::new();
        for (i, image) in self.drafts.get(draft).into_iter().flatten().enumerate() {
            let name = format!("pasted-image-{}.png", i + 1);
            let size = image.png.len();
            let upload = json!({ "type": "image", "name": name, "mimeType": "image/png", "sizeBytes": size });
            let sent = self.client.call("t3_attachment_prepare_upload", json!({ "upload": upload })).and_then(|prepared| {
                let id = str_of(&prepared["attachmentId"]).to_string();
                let url = str_of(&prepared["relativeUrl"]);
                if id.is_empty() || url.is_empty() {
                    return Err(T3Error::Other("T3 gave nowhere to upload it".into()));
                }
                if let Err(error) = self.client.upload(url, "image/png", &image.png) {
                    self.discard_uploads(&[json!({ "id": id })]);
                    return Err(error);
                }
                Ok(id)
            });
            match sent {
                Ok(id) => attachments.push(json!({ "type": "image", "id": id, "name": name, "mimeType": "image/png", "sizeBytes": size })),
                Err(error) => {
                    self.discard_uploads(&attachments);
                    return Err(T3Error::Other(format!("couldn't upload the image: {error}")));
                }
            }
        }
        Ok(attachments)
    }

    /// Gives back uploads a message didn't take (best effort: T3 drops them in time anyway).
    fn discard_uploads(&self, attachments: &[Value]) {
        for attachment in attachments {
            let _ = self.client.call("t3_attachment_discard", json!({ "attachmentId": attachment["id"] }));
        }
    }

    // ---- reading T3 ----

    /// Starts a poll: its steps run from `tick`, a few at a time.
    fn start_poll(&mut self) {
        let t = self.now_ms();
        let mut steps = VecDeque::new();
        if t >= self.slow_at {
            self.slow_at = t + 60_000;
            steps.push_back(Step::Machines);
        }
        steps.push_back(Step::List(0));
        steps.push_back(Step::View);
        self.poll_run = Some(PollRun { started: t, steps, lists: Vec::new(), listed: None, reading: None });
        // This poll covers whatever asked for one.
        self.repoll = false;
    }

    /// Past the poll's deadline, optional reads (question checks, launchers, projects) wait
    /// for the next poll.
    fn out_of_time(&self) -> bool {
        self.poll_run.as_ref().is_some_and(|run| self.now_ms() - run.started > POLL_DEADLINE_MS)
    }

    /// Puts steps next in the poll, in this order.
    fn push_steps(&mut self, steps: Vec<Step>) {
        if let Some(run) = self.poll_run.as_mut() {
            for step in steps.into_iter().rev() {
                run.steps.push_front(step);
            }
        }
    }

    /// Runs the poll's next step; the last one, or a failed one, ends the poll.
    fn poll_step(&mut self) {
        let Some(step) = self.poll_run.as_mut().and_then(|run| run.steps.pop_front()) else {
            return self.finish_poll(Ok(()));
        };
        let result = match step {
            Step::Machines => self.read_links(),
            Step::ServerName(links) => self.read_machines(links),
            Step::Projects(env) => {
                self.refresh_projects(&env);
                Ok(())
            }
            Step::Catalog(env) => {
                self.refresh_catalog(&env);
                Ok(())
            }
            Step::List(n) => {
                self.read_lists(n);
                Ok(())
            }
            Step::Machine(env, result) => self.take_lists(env, result),
            Step::Question(i) => self.check_question(i),
            Step::QuestionKind(i, read_at) => self.check_waiting_on(i, read_at),
            Step::Delegates => self.check_delegates(),
            Step::Watch(i) => self.check_watch(i),
            Step::Launcher(i) => self.find_launcher(i),
            Step::RunStart(i) => self.read_run_start(i),
            Step::Activity(i) => self.read_activity(i),
            Step::Track => {
                self.track_listed();
                Ok(())
            }
            Step::Unlisted(key, status) => self.check_unlisted(key, status),
            Step::Keep => {
                self.keep_lists();
                Ok(())
            }
            Step::View => self.refresh_view(),
            Step::ViewConfig => self.read_view_config(),
            Step::ViewPage => self.read_view_page(),
            Step::ViewAsk => self.read_view_ask(),
            Step::ViewAskRead(request_id) => self.read_view_request(request_id),
            Step::ViewScan => self.read_view_scan(),
            Step::ViewCard(item_id) => self.read_view_card(item_id),
            Step::ViewAnswer(position) => self.read_view_answer(position),
            Step::ViewChanges(item_id) => self.read_view_changes(item_id),
            Step::ViewDraw(i) => self.draw_picture(i),
            Step::ViewPictures => {
                self.place_pictures();
                Ok(())
            }
        };
        match result {
            Err(error) => self.finish_poll(Err(error)),
            Ok(()) if self.poll_run.as_ref().is_some_and(|run| run.steps.is_empty()) => self.finish_poll(Ok(())),
            Ok(()) => {}
        }
    }

    /// Ends the poll and schedules the next one from now: sooner with the window open,
    /// later after each failure in a row.
    fn finish_poll(&mut self, result: Result<(), T3Error>) {
        self.poll_run = None;
        let t = self.now_ms();
        let every = if self.window_open { 4000 } else { 12_000 };
        match result {
            Ok(()) => {
                self.polled_ok = true;
                self.poll_error = None;
                self.auth_error = None;
                self.poll_failures = 0;
                self.poll_at = t + every;
            }
            Err(T3Error::Auth(message)) => {
                if self.auth_error.as_ref() != Some(&message) {
                    self.log(&message);
                }
                self.auth_error = Some(message);
                self.poll_at = t + 30_000;
            }
            Err(T3Error::Other(message)) => {
                if self.poll_error.as_ref() != Some(&message) {
                    self.log(&format!("poll: {message}"));
                }
                self.poll_error = Some(message);
                self.poll_failures += 1;
                self.poll_at = t + (every << self.poll_failures.min(4)).min(60_000);
            }
        }
        // A command ran meanwhile: look again right away.
        if std::mem::take(&mut self.repoll) {
            self.poll_at = 0;
        }
    }

    /// A page of the `n`th of the three thread lists from every machine still answering,
    /// all at once. A machine whose list needs another page to reach its limit gets it in
    /// the next step; after the last list, each machine's lists are taken in, one machine
    /// after another.
    fn read_lists(&mut self, n: usize) {
        let mut lists = match self.poll_run.as_mut() {
            Some(run) if n > 0 || !run.lists.is_empty() => std::mem::take(&mut run.lists),
            Some(_) => self.envs.iter().filter(|e| e.ok).map(|e| (e.id.clone(), Ok(Vec::new()))).collect(),
            None => return,
        };
        let limit = self.list_limits[n];
        // Per machine, the page it needs now, if any: where it starts and how many threads.
        let args: Vec<Option<ListPage>> = lists
            .iter()
            .map(|(env, so_far)| {
                let page = match so_far {
                    Ok(replies) if replies.len() == n => Some((None, limit.min(LIST_PAGE))),
                    Ok(replies) => replies.get(n).and_then(|list| next_page(list, limit).map(|c| (Some(c), (limit - listed_count(list)).min(LIST_PAGE)))),
                    Err(_) => None,
                };
                page.map(|(cursor, count)| (self.env_arg(env), cursor, count))
            })
            .collect();
        let client = Arc::clone(&self.client);
        let replies: Vec<Option<Result<Value, T3Error>>> = std::thread::scope(|scope| {
            let handles: Vec<_> = args
                .iter()
                .map(|arg| {
                    let client = &client;
                    arg.as_ref().map(|(arg, cursor, count)| scope.spawn(move || fetch_list(client.as_ref(), n, *cursor, *count, arg)))
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.map(|h| h.join().unwrap_or_else(|_| Err(T3Error::Other("thread list panicked".into())))))
                .collect()
        });
        for ((_, so_far), reply) in lists.iter_mut().zip(replies) {
            match (reply, so_far.as_mut()) {
                // A later page: its threads join the list; its cursor and total replace.
                (Some(Ok(page)), Ok(replies)) if replies.len() > n => {
                    let list = &mut replies[n];
                    let threads = page["threads"].as_array().cloned().unwrap_or_default();
                    if let Some(all) = list["threads"].as_array_mut() {
                        all.extend(threads);
                    }
                    list["nextCursor"] = page["nextCursor"].clone();
                    list["total"] = page["total"].clone();
                }
                (Some(Ok(page)), Ok(replies)) => replies.push(page),
                (Some(Err(error)), _) => *so_far = Err(error),
                _ => {}
            }
        }
        let another_page = lists.iter().any(|(_, so_far)| so_far.as_ref().is_ok_and(|replies| replies.get(n).is_some_and(|list| next_page(list, limit).is_some())));
        if another_page || n + 1 < LIST_COUNT {
            if let Some(run) = self.poll_run.as_mut() {
                run.lists = lists;
                run.steps.push_front(Step::List(if another_page { n } else { n + 1 }));
            }
        } else {
            self.push_steps(lists.into_iter().map(|(env, so_far)| Step::Machine(env, so_far.map(|lists| combine_lists(&lists)))).collect());
        }
    }

    /// One machine's lists, as read: its question checks, launchers, status changes and
    /// threads that left the lists come next, a request at a time.
    fn take_lists(&mut self, env: String, result: ThreadLists) -> Result<(), T3Error> {
        let name = if env.is_empty() { "local".to_string() } else { env.clone() };
        match result {
            Ok((mut threads, settled_total, more)) => {
                threads.iter_mut().for_each(read_as_working_if_waiting);
                let mut steps: Vec<Step> = (0..threads.len()).filter(|i| working(str_of(&threads[*i]["status"]))).map(Step::Question).collect();
                steps.extend(threads.iter().any(run_over).then_some(Step::Delegates));
                steps.extend((0..threads.len()).filter(|i| may_watch(&threads[*i])).map(Step::Watch));
                steps.extend((0..threads.len()).filter(|i| threads[*i]["createdBy"] == "agent").map(Step::Launcher));
                steps.extend((0..threads.len()).filter(|i| working(str_of(&threads[*i]["status"]))).map(Step::RunStart));
                steps.extend((0..threads.len()).filter(|i| ages_from_run(&threads[*i])).map(Step::Activity));
                steps.extend([Step::Track, Step::Keep]);
                if let Some(run) = self.poll_run.as_mut() {
                    run.listed = Some(Listed { env, threads, settled_total, more, seen: Vec::new() });
                }
                self.push_steps(steps);
                Ok(())
            }
            Err(T3Error::Auth(message)) => Err(T3Error::Auth(message)),
            Err(T3Error::Other(message)) => {
                let local = self.local_env_id.clone();
                if let Some(target) = self.envs.iter_mut().find(|e| e.id == env && Some(&e.id) != local.as_ref()) {
                    target.ok = false;
                }
                // The server itself failing fails the poll; a linked machine is only marked.
                if Some(&env) == local.as_ref() || env.is_empty() {
                    return Err(T3Error::Other(message));
                }
                // Logged when it changes, not on every poll.
                if self.list_errors.insert(env.clone(), message.clone()).as_ref() != Some(&message) {
                    self.log(&format!("threads ({name}): {message}"));
                }
                Ok(())
            }
        }
    }

    /// The machine being taken in, its `i`th thread's id and its updatedAt.
    fn listed_thread(&self, i: usize) -> Option<(String, String, String)> {
        let listed = self.poll_run.as_ref()?.listed.as_ref()?;
        let thread = listed.threads.get(i)?;
        Some((listed.env.clone(), str_of(&thread["threadId"]).to_string(), str_of(&thread["updatedAt"]).to_string()))
    }

    /// The `i`th thread's item count, from its list.
    fn listed_item_count(&self, i: usize) -> u64 {
        let listed = self.poll_run.as_ref().and_then(|run| run.listed.as_ref());
        listed.and_then(|l| l.threads.get(i)).and_then(|t| t["itemCount"].as_u64()).unwrap_or(0)
    }

    fn listed_mut(&mut self) -> Option<&mut Listed> {
        self.poll_run.as_mut()?.listed.as_mut()
    }

    /// A working thread was looked at; with a status, it waits on that.
    fn mark_waiting(&mut self, i: usize, key: String, status: Option<&'static str>) {
        if let Some(listed) = self.listed_mut() {
            listed.seen.push(key);
            if let (Some(status), Some(thread)) = (status, listed.threads.get_mut(i)) {
                thread["status"] = json!(status);
            }
        }
    }

    /// A working thread whose agent waits on a question shows as "waiting" (Needs you),
    /// as in T3's sidebar, so it notifies and leaves the Working group; one waiting on a
    /// card only T3 can answer shows as CARD. The thread (its pending count and its latest
    /// few items) is read again only when it changed or the read is old.
    fn check_question(&mut self, i: usize) -> Result<(), T3Error> {
        let Some((env, id, updated)) = self.listed_thread(i) else { return Ok(()) };
        let key = format!("{env}:{id}");
        let now = self.now_ms();
        let known = match self.pending_counts.get(&key) {
            Some(p) if p.updated == updated && now - p.read_at < PENDING_RECHECK_MS => Some(p.status),
            // No time left in this poll: the last count stands until the next one.
            Some(p) if self.out_of_time() => Some(p.status),
            None if self.out_of_time() => Some(None),
            _ => None,
        };
        if let Some(status) = known {
            self.mark_waiting(i, key, status);
            return Ok(());
        }
        let args = with_args(pending_read_args(&id, self.listed_item_count(i)), &self.env_arg(&env));
        match self.client.call("t3_thread_read", args) {
            Ok(read) if read["thread"]["pendingRequestCount"].as_u64().unwrap_or(0) == 0 => {
                // The newest waiting card, as T3's sidebar picks it.
                let found = read["items"].as_array().into_iter().flatten().rev().find_map(|item| Some((waiting_card(item)?, item["itemId"].as_str().map(str::to_string))));
                let (card, request) = found.unzip();
                let status = card.is_some().then_some(CARD);
                self.pending_counts.insert(key.clone(), Pending { updated, read_at: now, status, card, request: request.flatten() });
                self.mark_waiting(i, key, status);
            }
            // Something's pending: what it is comes next.
            Ok(_) => self.push_steps(vec![Step::QuestionKind(i, now)]),
            Err(T3Error::Auth(message)) => return Err(T3Error::Auth(message)),
            Err(T3Error::Other(message)) => self.keep_last_waiting(i, key, &message),
        }
        Ok(())
    }

    /// A pending check that failed changes nothing: what the thread last waited on stands.
    fn keep_last_waiting(&mut self, i: usize, key: String, message: &str) {
        self.log(&format!("pending check ({key}): {message}"));
        let status = self.pending_counts.get(&key).and_then(|p| p.status);
        self.mark_waiting(i, key, status);
    }

    /// What a working thread with pending requests waits on: its count includes approvals,
    /// which T3's pending list leaves out, so a count with nothing listed is an approval.
    fn check_waiting_on(&mut self, i: usize, read_at: i64) -> Result<(), T3Error> {
        let Some((env, id, updated)) = self.listed_thread(i) else { return Ok(()) };
        let key = format!("{env}:{id}");
        let args = with_args(json!({ "action": "list", "threadId": id }), &self.env_arg(&env));
        match self.client.call("t3_pending_request", args) {
            Ok(list) => {
                // Oldest first: the newest is the one T3 notifies for.
                let request = list["requestIds"].as_array().and_then(|ids| ids.last()).and_then(Value::as_str).map(str::to_string);
                let status = Some(if request.is_some() { "waiting" } else { "approval" });
                self.pending_counts.insert(key.clone(), Pending { updated, read_at, status, card: None, request });
                self.mark_waiting(i, key, status);
            }
            Err(T3Error::Auth(message)) => return Err(T3Error::Auth(message)),
            Err(T3Error::Other(message)) => self.keep_last_waiting(i, key, &message),
        }
        Ok(())
    }

    /// A thread whose run ended with a subagent still going is waiting on it (a delegated
    /// review, say): it shows as "watching" until T3 wakes it. No time left in this poll, or
    /// no answer: the last one stands.
    fn check_delegates(&mut self) -> Result<(), T3Error> {
        let Some(env) = self.listed_mut().map(|l| l.env.clone()) else { return Ok(()) };
        if !self.out_of_time() {
            let statuses = ["preparing", "queued", "starting", "running", "waiting"];
            let args = with_args(json!({ "allProjects": true, "includeSubagents": true, "statuses": statuses, "limit": 100 }), &self.env_arg(&env));
            match self.client.call("t3_thread_list", args) {
                Ok(list) => {
                    self.delegating.insert(env.clone(), waiting_parents(&list));
                }
                Err(T3Error::Auth(message)) => return Err(T3Error::Auth(message)),
                Err(T3Error::Other(message)) => self.log(&format!("subagents ({env}): {message}")),
            }
        }
        let parents = self.delegating.get(&env).cloned().unwrap_or_default();
        if let Some(listed) = self.listed_mut() {
            for thread in listed.threads.iter_mut().filter(|t| run_over(t) && parents.contains(str_of(&t["threadId"]))) {
                thread["status"] = json!(WATCHING);
            }
        }
        Ok(())
    }

    /// A thread whose run ended with a pull request still open may be watching it (its
    /// checks, its reviews): then it shows as "watching", under Working as in T3's sidebar.
    /// Read again only when the thread changed or the answer is old.
    fn check_watch(&mut self, i: usize) -> Result<(), T3Error> {
        let Some((env, id, updated)) = self.listed_thread(i) else { return Ok(()) };
        let key = format!("{env}:{id}");
        let now = self.now_ms();
        let known = match self.watches.get(&key) {
            Some(w) if w.updated == updated && now - w.read_at < WATCH_RECHECK_MS => Some(w.watching),
            // No time left in this poll: the last answer stands until the next one.
            Some(w) if self.out_of_time() => Some(w.watching),
            None if self.out_of_time() => Some(false),
            _ => None,
        };
        let watching = match known {
            Some(watching) => watching,
            None => {
                let args = with_args(json!({ "threadId": id }), &self.env_arg(&env));
                match self.client.call("list_thread_pull_requests", args) {
                    Ok(list) => {
                        let watching = list["pullRequests"].as_array().is_some_and(|prs| prs.iter().any(|pr| pr["watching"] == true));
                        self.watches.insert(key, Watch { updated, read_at: now, watching });
                        watching
                    }
                    Err(T3Error::Auth(message)) => return Err(T3Error::Auth(message)),
                    // Unreadable now: the last answer stands.
                    Err(T3Error::Other(message)) => {
                        self.log(&format!("pull request watch ({key}): {message}"));
                        self.watches.get(&key).is_some_and(|w| w.watching)
                    }
                }
            }
        };
        if watching {
            if let Some(thread) = self.listed_mut().and_then(|l| l.threads.get_mut(i)) {
                thread["status"] = json!(WATCHING);
            }
        }
        Ok(())
    }

    /// Notes when a working thread's current run started, for the time beside its status.
    fn read_run_start(&mut self, i: usize) -> Result<(), T3Error> {
        let Some((env, id, _)) = self.listed_thread(i) else { return Ok(()) };
        let run = self.listed_mut().and_then(|l| l.threads.get(i)).map(|t| str_of(&t["latestRunId"]).to_string()).unwrap_or_default();
        if run.is_empty() {
            return Ok(());
        }
        let key = format!("{env}:{id}");
        if self.run_starts.get(&key).is_none_or(|(known, _)| *known != run) && !self.out_of_time() {
            let args = with_args(json!({ "threadId": id, "limit": 1, "maxCharsPerItem": 1, "runLimit": 1 }), &self.env_arg(&env));
            match self.client.call("t3_thread_read", args) {
                Ok(read) => {
                    let started = read["recentRuns"].as_array().and_then(|runs| runs.iter().find(|r| r["runId"] == run.as_str())).and_then(|r| r["startedAt"].as_str()).map(iso_seconds);
                    if let Some(started) = started.filter(|s| *s > 0) {
                        self.run_starts.insert(key.clone(), (run.clone(), started));
                    }
                }
                Err(T3Error::Auth(message)) => return Err(T3Error::Auth(message)),
                Err(T3Error::Other(message)) => self.log(&format!("run start ({key}): {message}")),
            }
        }
        if let Some(started) = self.run_starts.get(&key).filter(|(known, _)| *known == run).map(|(_, started)| *started) {
            if let Some(thread) = self.listed_mut().and_then(|l| l.threads.get_mut(i)) {
                thread["runStartedAt"] = json!(started);
            }
        }
        Ok(())
    }

    /// Notes when a thread at rest last had activity, which T3's sidebar ages its row from:
    /// the newest of its latest run's times. T3's list doesn't say, so it's read again only
    /// when the thread's updatedAt moves; until then the row ages from updatedAt.
    fn read_activity(&mut self, i: usize) -> Result<(), T3Error> {
        let Some((env, id, updated)) = self.listed_thread(i) else { return Ok(()) };
        let key = format!("{env}:{id}");
        if self.activity.get(&key).is_none_or(|(known, _)| *known != updated) && !self.out_of_time() {
            let args = with_args(json!({ "threadId": id, "limit": 1, "maxCharsPerItem": 1, "runLimit": 1 }), &self.env_arg(&env));
            match self.client.call("t3_thread_read", args) {
                Ok(read) => {
                    let at = read["recentRuns"].as_array().and_then(|runs| runs.first()).map_or(0, run_activity);
                    if at > 0 {
                        self.activity.insert(key.clone(), (updated.clone(), at));
                    }
                }
                Err(T3Error::Auth(message)) => return Err(T3Error::Auth(message)),
                Err(T3Error::Other(message)) => self.log(&format!("activity ({key}): {message}")),
            }
        }
        if let Some(at) = self.activity.get(&key).filter(|(known, _)| *known == updated).map(|(_, at)| *at) {
            if let Some(thread) = self.listed_mut().and_then(|l| l.threads.get_mut(i)) {
                thread["activityAt"] = json!(at);
            }
        }
        Ok(())
    }

    /// Notes where an agent-made thread came from: the thread that launched it (the sender
    /// of its first message), else whether T3 lists it.
    ///
    /// T3's sidebar leaves out a task delegated from another machine, by a field
    /// (`delegatedFrom`) its tools don't return. Such a task names no sender. One given a
    /// role opens with it (`opens_delegated_task`); one without opens with the brief alone,
    /// and runs in the project's checkout, with no branch of its own, usually once. So a
    /// thread with no role line is taken as listed when it has a branch (launched with a
    /// worktree) or ran again (talked to), and for a delegated task otherwise, read again
    /// once it changes. That gets two cases wrong: a role-less delegated task that ran again
    /// shows, and a thread launched into a project's checkout, from another machine or by an
    /// outside MCP client, stays out of Open while idle until its second run.
    fn find_launcher(&mut self, i: usize) -> Result<(), T3Error> {
        let Some((env, id, updated)) = self.listed_thread(i) else { return Ok(()) };
        let key = format!("{env}:{id}");
        let stale = match self.launchers.get(&key) {
            None => true,
            Some(origin) => origin.open && origin.updated != updated,
        };
        if stale && !self.out_of_time() {
            let args = with_args(json!({ "threadId": id, "view": "messages", "limit": 1, "maxCharsPerItem": OPENING_CHARS }), &self.env_arg(&env));
            match self.client.call("t3_thread_read", args) {
                Ok(read) => {
                    let first = &read["items"][0];
                    let launcher = first["senderThreadId"].as_str().filter(|s| *s != id).map(str::to_string);
                    let thread = &read["thread"];
                    let role = opens_delegated_task(str_of(&first["text"]));
                    let looks_listed = thread["branch"].is_string() || thread["runCount"].as_u64().unwrap_or(0) > 1;
                    let listed = launcher.is_none() && !role && looks_listed;
                    let open = launcher.is_none() && !role && !listed;
                    self.launchers.insert(key.clone(), Origin { launcher, listed, open, updated });
                }
                Err(T3Error::Auth(message)) => return Err(T3Error::Auth(message)),
                Err(T3Error::Other(message)) => self.log(&format!("launcher ({key}): {message}")),
            }
        }
        if let Some(origin) = self.launchers.get(&key).cloned() {
            if let Some(thread) = self.listed_mut().and_then(|l| l.threads.get_mut(i)) {
                match origin.launcher {
                    Some(launcher) => thread["launcher"] = json!(launcher),
                    None if origin.listed => thread["ownRow"] = json!(true),
                    None => {}
                }
            }
        }
        Ok(())
    }

    /// The machine's status changes notify, the pending counts of working threads not
    /// looked at go, and each active thread that left its lists is read next.
    fn track_listed(&mut self) {
        let Some(listed) = self.poll_run.as_mut().and_then(|run| run.listed.take()) else { return };
        for thread in &listed.threads {
            self.track_status(&listed.env, thread);
        }
        let prefix = format!("{}:", listed.env);
        self.pending_counts.retain(|k, _| !k.starts_with(&prefix) || listed.seen.contains(k));
        let ids: HashSet<String> = listed.threads.iter().map(|t| format!("{prefix}{}", str_of(&t["threadId"]))).collect();
        self.unlisted_misses.retain(|k, _| !k.starts_with(&prefix) || !ids.contains(k));
        self.watches.retain(|k, _| !k.starts_with(&prefix) || ids.contains(k));
        self.run_starts.retain(|k, _| !k.starts_with(&prefix) || ids.contains(k));
        self.activity.retain(|k, _| !k.starts_with(&prefix) || ids.contains(k));
        let gone: Vec<(String, String)> =
            self.last_status.iter().filter(|(k, _)| k.starts_with(&prefix) && !ids.contains(*k)).map(|(k, s)| (k.clone(), s.clone())).collect();
        let mut steps = Vec::new();
        for (key, status) in gone {
            if working(&status) || needs_you(&status) {
                steps.push(Step::Unlisted(key, status));
            } else {
                self.last_status.remove(&key);
                self.unlisted_misses.remove(&key);
            }
        }
        let last_status = &self.last_status;
        self.shown_requests.retain(|k, _| last_status.contains_key(k));
        if let Some(run) = self.poll_run.as_mut() {
            run.listed = Some(listed);
        }
        self.push_steps(steps);
    }

    /// A thread that was working or waiting on you and dropped off its machine's lists
    /// (settled past the list's limits, say) is read once more each poll until it ends, so
    /// its end still notifies. Every other thread no longer listed is forgotten. A poll out
    /// of time skips the read and keeps watching; a failed read keeps watching too, until
    /// UNLISTED_MISSES in a row (a deleted thread).
    fn check_unlisted(&mut self, key: String, status: String) -> Result<(), T3Error> {
        let Some(env) = self.poll_run.as_ref().and_then(|run| run.listed.as_ref()).map(|l| l.env.clone()) else { return Ok(()) };
        let keep = if self.out_of_time() {
            true
        } else {
            let id = key.get(env.len() + 1..).unwrap_or("");
            let args = with_args(json!({ "threadId": id, "limit": 1, "maxCharsPerItem": 1 }), &self.env_arg(&env));
            match self.client.call("t3_thread_read", args) {
                Ok(read) if read["thread"].is_object() => {
                    self.unlisted_misses.remove(&key);
                    let mut thread = read["thread"].clone();
                    read_as_working_if_waiting(&mut thread);
                    if working(str_of(&thread["status"])) && thread["pendingRequestCount"].as_u64().unwrap_or(0) > 0 {
                        thread["status"] = json!(if needs_you(&status) { status.as_str() } else { "waiting" });
                    }
                    self.track_status(&env, &thread);
                    working(str_of(&thread["status"])) || needs_you(str_of(&thread["status"]))
                }
                Err(T3Error::Auth(message)) => return Err(T3Error::Auth(message)),
                // Unreadable now (T3 busy, or the thread deleted): try again next poll, a
                // few times.
                _ => {
                    let misses = self.unlisted_misses.entry(key.clone()).or_insert(0);
                    *misses += 1;
                    *misses < UNLISTED_MISSES
                }
            }
        };
        if !keep {
            self.last_status.remove(&key);
            self.unlisted_misses.remove(&key);
        }
        Ok(())
    }

    /// The machine's lists, taken in: they show from now on.
    fn keep_lists(&mut self) {
        let Some(Listed { env, threads, settled_total, more, .. }) = self.poll_run.as_mut().and_then(|run| run.listed.take()) else { return };
        self.listed_envs.insert(env.clone());
        self.threads.insert(env.clone(), threads);
        self.settled_totals.insert(env.clone(), settled_total);
        self.list_more.insert(env.clone(), more);
        if let Some(target) = self.envs.iter_mut().find(|e| e.id == env) {
            target.ok = true;
        }
        if self.list_errors.remove(&env).is_some() {
            let name = if env.is_empty() { "local" } else { &env };
            self.log(&format!("threads ({name}): reachable again"));
        }
    }

    /// The machines T3 links (the server's own name is read next).
    fn read_links(&mut self) -> Result<(), T3Error> {
        let result = self.client.call("t3_environment_links", json!({}))?;
        let local_env_id = result["environmentId"].as_str().map(str::to_string);
        // Another server behind the same sign-in (another T3 now runs on this PC): a new
        // sign-in as far as the game goes. This poll is already the new server's: it stays.
        if self.local_env_id.is_some() && local_env_id != self.local_env_id {
            self.new_login(&format!("T3 is now {}", local_env_id.as_deref().unwrap_or("another server")));
        }
        self.local_env_id = local_env_id;
        self.push_steps(vec![Step::ServerName(result)]);
        Ok(())
    }

    /// The server's own name (not this PC's when `login --origin` points at another
    /// machine), then the machines as T3 links them now; each one's projects come next.
    fn read_machines(&mut self, links: Value) -> Result<(), T3Error> {
        let name = match self.client.call("t3_environment_read", json!({})) {
            Ok(read) => {
                let local = self.local_env_id.clone().unwrap_or_default();
                self.new_worktrees.insert(local.clone(), read["preferences"]["defaultThreadEnvMode"] == "worktree");
                self.note_fresh_context(&local, &read);
                self.note_t3_version(&read);
                read["label"].as_str().filter(|l| !l.is_empty()).map(str::to_string)
            }
            Err(T3Error::Auth(message)) => return Err(T3Error::Auth(message)),
            Err(T3Error::Other(_)) => None,
        };
        let mut envs = vec![Env { id: self.local_env_id.clone().unwrap_or_default(), name: name.unwrap_or_else(hostname), ok: true }];
        for link in links["links"].as_array().into_iter().flatten() {
            let id = str_of(&link["environmentId"]).to_string();
            let name = link["label"].as_str().map(str::to_string).unwrap_or_else(|| id.chars().take(8).collect());
            envs.push(Env { id, name, ok: link["status"] == "reachable" });
        }
        self.envs = envs;
        self.forget_unlinked();
        let now = self.now_ms();
        let mut steps = Vec::new();
        for env in self.envs.iter().filter(|e| e.ok) {
            steps.push(Step::Projects(env.id.clone()));
            if self.catalogs.get(&env.id).is_none_or(|c| now - c.read_at >= CATALOG_TTL_MS) {
                steps.push(Step::Catalog(env.id.clone()));
            }
        }
        self.push_steps(steps);
        Ok(())
    }

    /// Drops what was read from machines T3 no longer links (their threads would otherwise
    /// stay listed forever, under another machine's name).
    fn forget_unlinked(&mut self) {
        let known: HashSet<String> = self.envs.iter().map(|e| e.id.clone()).collect();
        let linked = |env: &str| known.contains(env);
        // A thread opened before the server's own id was known.
        if let Some(view) = self.view.as_mut().filter(|v| v.env.is_empty()) {
            view.env = self.local_env_id.clone().unwrap_or_default();
        }
        self.threads.retain(|env, _| linked(env));
        self.projects.retain(|env, _| linked(env));
        self.new_worktrees.retain(|env, _| linked(env));
        self.fresh_context.retain(|env| linked(env));
        self.catalogs.retain(|env, _| linked(env));
        self.settled_totals.retain(|env, _| linked(env));
        self.list_more.retain(|env, _| linked(env));
        self.listed_envs.retain(|env| linked(env));
        self.last_status.retain(|key, _| linked(env_of(key)));
        self.shown_requests.retain(|key, _| linked(env_of(key)));
        self.unlisted_misses.retain(|key, _| linked(env_of(key)));
        self.pending_counts.retain(|key, _| linked(env_of(key)));
        self.watches.retain(|key, _| linked(env_of(key)));
        self.launchers.retain(|key, _| linked(env_of(key)));
        self.delegating.retain(|env, _| linked(env));
        self.list_errors.retain(|env, _| linked(env));
        if self.view.as_ref().is_some_and(|v| !linked(&v.env)) {
            self.view = None;
            self.view_data = None;
            self.view_key = None;
            self.pending_ask = None;
        }
    }

    /// Notes from a machine's `t3_environment_read` whether its T3 starts fresh sessions.
    fn note_fresh_context(&mut self, env: &str, read: &Value) {
        if takes_fresh_context(str_of(&read["serverVersion"])) {
            self.fresh_context.insert(env.to_string());
        } else {
            self.fresh_context.remove(env);
        }
    }

    /// Notes from the server's `t3_environment_read` whether it runs an older T3 Code than
    /// this release is made for, and logs each change.
    fn note_t3_version(&mut self, read: &Value) {
        let older = older_t3(str_of(&read["serverVersion"]));
        if older != self.older_t3 {
            match &older {
                Some(older) => self.log(&format!("T3 runs T3 Code {}, older than the {} this release is made for", older.running, made_for::T3_CODE)),
                None if self.older_t3.is_some() => self.log(&format!("T3 now runs T3 Code {} or later", made_for::T3_CODE)),
                None => {}
            }
            self.older_t3 = older;
        }
    }

    /// Whether a send to this machine can start a fresh session: the T3 it goes through (this
    /// server, which passes a linked machine's on) has to take `freshContext` too.
    fn fresh_context_on(&self, env: &str) -> bool {
        let local = self.local_env_id.as_deref().unwrap_or_default();
        self.fresh_context.contains(local) && self.fresh_context.contains(env)
    }

    fn refresh_projects(&mut self, env: &str) {
        if self.out_of_time() {
            return;
        }
        if self.env_arg(env).is_empty() {
            self.read_t3_settings();
        }
        let args = with_args(json!({ "limit": 100 }), &self.env_arg(env));
        match self.client.call("t3_project_list", args) {
            Ok(page) => {
                self.projects.insert(env.to_string(), page["projects"].as_array().cloned().unwrap_or_default());
            }
            Err(error) => self.log(&format!("projects ({}): {error}", if env.is_empty() { "local" } else { env })),
        }
        // A linked machine's preference, read with its projects (the server's own comes with its name).
        let env_arg = self.env_arg(env);
        if !env_arg.is_empty() && !self.out_of_time() {
            match self.client.call("t3_environment_read", Value::Object(env_arg)) {
                Ok(read) => {
                    self.new_worktrees.insert(env.to_string(), read["preferences"]["defaultThreadEnvMode"] == "worktree");
                    self.note_fresh_context(env, &read);
                }
                Err(error) => self.log(&format!("preferences ({env}): {error}")),
            }
        }
    }

    /// What T3 lists for a project. None when that can't be known: T3 works it out from the
    /// settings of the machine the project is on, which only this PC's disk shows, and its
    /// tools return only the project's own list, which those settings usually replace.
    fn project_scripts(&self, env: &str, project: &str) -> Option<&[Value]> {
        static NO_SETTINGS: Value = Value::Null;
        self.t3_settings.as_ref().filter(|_| self.env_arg(env).is_empty())?;
        let own = self.projects.get(env).and_then(|list| list.iter().find(|p| p["id"] == project)).and_then(|p| p["scripts"].as_array()).map_or(&[][..], Vec::as_slice);
        Some(resolved_scripts(self.script_settings.as_ref().unwrap_or(&NO_SETTINGS), project, own))
    }

    /// The parts of T3's settings that say which scripts this PC's projects have, and which
    /// snooze presets T3 offers (T3 writes them only once changed). Settings that can't be
    /// read just now (T3 may be rewriting them) keep what they last said.
    fn read_t3_settings(&mut self) {
        let Some(file) = &self.t3_settings else { return };
        let Some(Value::Object(mut settings)) = std::fs::read_to_string(file).ok().and_then(|text| serde_json::from_str(&text).ok()) else { return };
        self.snooze_presets = settings.get("snoozePresets").and_then(Value::as_array).cloned();
        settings.retain(|key, _| SCRIPT_SETTINGS.contains(&key.as_str()));
        self.script_settings = Some(Value::Object(settings));
    }

    /// The snooze presets the game offers: T3's, from its settings when this PC can read
    /// them (T3's defaults when they list none), else the four the game always had.
    fn snooze_choices(&self) -> Vec<crate::snooze::Choice> {
        let presets = match &self.t3_settings {
            None => crate::snooze::fixed_presets(),
            Some(_) => self.snooze_presets.clone().unwrap_or_else(crate::snooze::t3_default_presets),
        };
        crate::snooze::choices(&presets, self.now_seconds(), &*self.zone)
    }

    /// What a machine offers. A failed read keeps the last one, and waits as long as a good
    /// one would: an older T3 that lacks the tool shouldn't be asked every minute.
    fn refresh_catalog(&mut self, env: &str) {
        if self.out_of_time() {
            return;
        }
        let read_at = self.now_ms();
        match self.client.call("orchestrator_capabilities", Value::Object(self.env_arg(env))) {
            Ok(capabilities) => {
                let mode = capabilities["runtimeMode"].as_str().map(str::to_string);
                let providers = compact_providers(&capabilities);
                let picks = self.read_picks(env, &providers);
                self.catalogs.insert(env.to_string(), Catalog { read_at, providers, mode, picks });
            }
            Err(error) => {
                self.log(&format!("models ({}): {error}", if env.is_empty() { "local" } else { env }));
                self.catalogs.entry(env.to_string()).or_default().read_at = read_at;
            }
        }
    }

    /// What this PC's providers list in their composers. A cache that can't be read just now
    /// (T3 may be rewriting it) keeps what it last listed.
    fn read_picks(&self, env: &str, providers: &[Value]) -> Vec<Value> {
        let Some(dir) = self.provider_caches.as_ref().filter(|_| self.env_arg(env).is_empty()) else {
            return Vec::new();
        };
        let known = self.catalogs.get(env).map_or(&[][..], |catalog| catalog.picks.as_slice());
        let mut picks = Vec::new();
        for provider in providers {
            let id = str_of(&provider["id"]);
            // It names a file, so nothing that could leave the folder.
            if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || "_-".contains(c)) {
                continue;
            }
            let cache = std::fs::read_to_string(dir.join(format!("{id}.json"))).ok().and_then(|text| serde_json::from_str::<Value>(&text).ok());
            match cache {
                Some(cache) => {
                    let listed = compact_picks(&cache);
                    if !listed.is_empty() {
                        picks.push(json!({ "pv": id, "c": listed }));
                    }
                }
                None => picks.extend(known.iter().find(|pick| pick["pv"] == id).cloned()),
            }
        }
        picks
    }

    /// A model as the game names it: what the machine calls it, else its id.
    fn model_name(&self, env: &str, provider: &Value, model: &Value) -> String {
        let listed = self
            .catalogs
            .get(env)
            .and_then(|c| c.providers.iter().find(|p| p["id"] == *provider))
            .and_then(|p| p["m"].as_array()?.iter().find(|m| m["id"] == *model));
        listed.map_or_else(|| plain(model_text(str_of(model), None), 40), |m| str_of(&m["t"]).to_string())
    }

    /// Where a thread moved to another provider, by the position of the first message on it:
    /// what the line over that message says, as T3 draws one. Runs carry the provider and
    /// model, and every message names its run; messages from runs too old to be listed are
    /// passed over.
    fn handoffs(&self, env: &str, messages: &[Value], runs: &Value) -> HashMap<u64, String> {
        let mut lines = HashMap::new();
        let mut previous: Option<&Value> = None;
        let runs = runs.as_array().map_or(&[][..], Vec::as_slice);
        for message in messages.iter().filter(|m| matches!(str_of(&m["type"]), "user_message" | "assistant_message")) {
            let Some(run) = runs.iter().find(|r| r["runId"] == message["runId"]) else { continue };
            if let (Some(before), Some(position)) = (previous, message["position"].as_u64()) {
                if before["providerInstanceId"] != run["providerInstanceId"] {
                    let name = |r: &Value| self.model_name(env, &r["providerInstanceId"], &r["model"]);
                    lines.insert(position, format!("{} > {}", name(before), name(run)));
                }
            }
            previous = Some(run);
        }
        lines
    }

    /// The model a command picked, as T3 takes it (none picked: None), with `kept` options and
    /// its effort. T3 stores whatever effort it's given, so one it doesn't offer stops here.
    fn model_selection(&self, env: &str, command: &Value, mut kept: Vec<Value>) -> Result<Option<Value>, T3Error> {
        let model = str_of(&command["md"]);
        if model.is_empty() {
            return Ok(None);
        }
        let provider = str_of(&command["pv"]);
        let effort = str_of(&command["ef"]);
        if !effort.is_empty() {
            let listed = self
                .catalogs
                .get(env)
                .and_then(|c| c.providers.iter().find(|p| p["id"] == provider))
                .and_then(|p| p["m"].as_array()?.iter().find(|m| m["id"] == model));
            let option = listed
                .filter(|m| m["ef"].as_array().is_some_and(|values| values.iter().any(|v| v == effort)))
                .map(|m| str_of(&m["o"]).to_string())
                .ok_or_else(|| T3Error::Other(format!("{model} has no effort \"{}\"", plain(effort, 20))))?;
            kept.retain(|o| o["id"] != option.as_str());
            kept.push(json!({ "id": option, "value": effort }));
        }
        let mut selection = json!({ "instanceId": provider, "model": model });
        if !kept.is_empty() {
            selection["options"] = json!(kept);
        }
        Ok(Some(selection))
    }

    fn track_status(&mut self, env: &str, thread: &Value) {
        let key = format!("{env}:{}", str_of(&thread["threadId"]));
        let status = str_of(&thread["status"]).to_string();
        if !working(&status) && self.stopping.remove(&key).is_some() {
            self.note("Stopped.");
        }
        let before = self.last_status.insert(key.clone(), status.clone());
        // Each request a thread waits on notifies once, until the thread stops waiting on you
        // (as in T3): a second question that replaces the first notifies, and the first, back
        // when the second is answered, doesn't again. An approval has no id: its status rules.
        let request = needs_you(&status).then(|| self.pending_counts.get(&key).and_then(|p| p.request.clone())).flatten();
        let new_request = match request {
            Some(request) => self.shown_requests.entry(key.clone()).or_default().insert(request),
            None => false,
        };
        if !needs_you(&status) {
            self.shown_requests.remove(&key);
        }
        let notify = match before.as_deref() {
            // A thread new since the last list that already waits on you (an agent that
            // asked straight away) notifies too; on the first list everything is old news.
            None => needs_you(&status) && self.listed_envs.contains(env),
            Some(before) if before == status => new_request,
            // Stopped working: finished, failed, stopped, or now waits on you. A run that
            // ended watching its pull request isn't done yet: it notifies when that ends.
            Some(before) if active(before) => !active(&status),
            // Answered and done before the next look (it never showed as working again).
            Some(before) if needs_you(before) => !active(&status) && !needs_you(&status),
            Some(_) => false,
        };
        if notify {
            self.event_seq = (self.event_seq + 1).max(self.now_ms());
            self.events.push(json!({
                "n": self.event_seq,
                "e": env,
                "id": str_of(&thread["threadId"]),
                "t": plain(str_of(&thread["title"]), 80),
                "s": status,
            }));
            let excess = self.events.len().saturating_sub(10);
            self.events.drain(..excess);
        }
    }

    /// A command changed what the open thread shows: read it again, and drop a read of it
    /// under way (it would bring back what the command just changed).
    fn view_changed(&mut self) {
        self.view_key = None;
        self.view_gen += 1;
    }

    /// The open thread's head, when it may have changed; its messages and question are read
    /// next, a request at a time. Its messages: only what came after the last one read when
    /// that's safe (same thread and depth, no items gone, a run still going or just started,
    /// read recently), re-reading that last one, which may still be growing. Otherwise a
    /// full read of the last 80×depth items, widened until there are enough.
    fn refresh_view(&mut self) -> Result<(), T3Error> {
        let Some(view) = self.view.clone() else {
            self.view_data = None;
            self.view_cache = None;
            return Ok(());
        };
        let listed = self.threads.get(&view.env).and_then(|list| list.iter().find(|t| t["threadId"] == view.id.as_str())).cloned();
        // A thread past the lists' limits has no updatedAt to compare: look again now and then.
        let freshness = match &listed {
            Some(t) => format!("{}:{}", str_of(&t["updatedAt"]), str_of(&t["status"])),
            None => format!("unlisted:{}", self.now_ms().div_euclid(UNLISTED_VIEW_TTL_MS)),
        };
        let key = format!("{}:{}:{}:{freshness}", view.env, view.id, view.depth);
        if Some(&key) == self.view_key.as_ref() && self.view_data.is_some() {
            return Ok(());
        }
        let args = with_args(json!({ "threadId": view.id, "limit": 1, "maxCharsPerItem": 1, "runLimit": 50 }), &self.env_arg(&view.env));
        let head = self.client.call("t3_thread_read", args)?;
        let mut thread = head["thread"].clone();
        read_as_working_if_waiting(&mut thread);
        let runs = head["recentRuns"].clone();
        let item_count = thread["itemCount"].as_u64().unwrap_or(0) as usize;
        let status = str_of(&thread["status"]);
        let now = self.now_ms();
        let cache = self.view_cache.take().filter(|c| {
            c.env == view.env
                && c.id == view.id
                && c.depth == view.depth
                && item_count >= c.item_count
                && now - c.read_at < VIEW_FULL_READ_MS
                // A run that ended may have tidied its messages.
                && (!working(&c.status) || working(status))
        });
        let last = cache.as_ref().and_then(|c| c.messages.last()).and_then(|m| m["position"].as_u64());
        let (cache, after, start, window) = match last {
            Some(last) => (cache, last.checked_sub(1), 0, 0),
            None => {
                let window = 80 * view.depth;
                let start = item_count.saturating_sub(window);
                (None, (start > 0).then_some(start as u64), start, window)
            }
        };
        let gen = self.view_gen;
        if let Some(run) = self.poll_run.as_mut() {
            run.reading = Some(ViewRead {
                view,
                key,
                gen,
                thread,
                config: None,
                runs,
                began: now,
                cache,
                messages: Vec::new(),
                after,
                pages: 0,
                start,
                window,
                passes: 0,
                built: None,
                positions: Vec::new(),
                embedded: Vec::new(),
                pictures: Vec::new(),
                drawing: Vec::new(),
                scan_pages: 0,
                again: false,
            });
        }
        self.push_steps(vec![Step::ViewConfig, Step::ViewPage]);
        Ok(())
    }

    /// What the open thread runs on, for the selectors. The conversation doesn't wait on it:
    /// a T3 that doesn't answer leaves the view with what the thread's read says.
    fn read_view_config(&mut self) -> Result<(), T3Error> {
        let Some(read) = self.view_read() else { return Ok(()) };
        let args = json!({ "threadId": read.view.id });
        let env = read.view.env.clone();
        let config = match self.client.call("t3_thread_configure", with_args(args, &self.env_arg(&env))) {
            Ok(config) => Some(config),
            Err(T3Error::Auth(message)) => return Err(T3Error::Auth(message)),
            Err(T3Error::Other(error)) => {
                self.log(&format!("thread settings: {error}"));
                None
            }
        };
        if let Some(read) = self.view_read() {
            read.config = config;
        }
        Ok(())
    }

    /// The open thread's read under way, unless a command changed the view since it began:
    /// that read is dropped (the poll after this one reads the view again).
    fn view_read(&mut self) -> Option<&mut ViewRead> {
        let gen = self.view_gen;
        let run = self.poll_run.as_mut()?;
        if run.reading.as_ref().is_some_and(|r| r.gen != gen) {
            let dropped = run.reading.take().and_then(|r| r.cache);
            if self.view_cache.is_none() {
                self.view_cache = dropped;
            }
            return None;
        }
        self.poll_run.as_mut()?.reading.as_mut()
    }

    /// One page of the open thread's messages, VIEW_PAGES at most a pass. A full read that
    /// found too few widens, VIEW_PASSES at most; then the view for the game is built.
    fn read_view_page(&mut self) -> Result<(), T3Error> {
        let Some(read) = self.view_read() else { return Ok(()) };
        let mut args = json!({ "threadId": read.view.id, "view": "messages", "limit": 100, "maxCharsPerItem": 6000 });
        if let Some(after) = read.after {
            args["afterPosition"] = json!(after);
        }
        let env = read.view.env.clone();
        let result = self.client.call("t3_thread_read", with_args(args, &self.env_arg(&env)))?;
        let Some(read) = self.view_read() else { return Ok(()) };
        read.messages.extend(result["items"].as_array().cloned().unwrap_or_default());
        read.pages += 1;
        let next = match (result["hasMore"] == true, result["nextPosition"].as_u64()) {
            (true, Some(next)) if read.pages < VIEW_PAGES => Some(next),
            _ => None,
        };
        let widen = next.is_none() && read.cache.is_none() && {
            read.passes += 1;
            read.messages.len() < VIEW_MESSAGES * read.view.depth && read.start > 0 && read.passes < VIEW_PASSES
        };
        if let Some(next) = next {
            read.after = Some(next);
        } else if widen {
            read.window *= 4;
            read.start = (read.thread["itemCount"].as_u64().unwrap_or(0) as usize).saturating_sub(read.window);
            read.after = (read.start > 0).then_some(read.start as u64);
            read.messages.clear();
            read.pages = 0;
        } else {
            self.build_view();
            return Ok(());
        }
        self.push_steps(vec![Step::ViewPage]);
        Ok(())
    }

    /// The view for the game, from the messages read; its question is read next, if it has one.
    fn build_view(&mut self) {
        let Some(mut read) = self.poll_run.as_mut().and_then(|run| run.reading.take()) else { return };
        let View { env, id, depth } = read.view.clone();
        let newer = std::mem::take(&mut read.messages);
        let (messages, earlier, read_at) = match read.cache.take() {
            Some(cache) => {
                let last = cache.messages.last().and_then(|m| m["position"].as_u64()).unwrap_or(0);
                let mut messages: Vec<Value> = cache.messages.into_iter().filter(|m| m["position"].as_u64().is_some_and(|p| p < last)).collect();
                messages.extend(newer);
                (messages, cache.earlier, cache.read_at)
            }
            None => (newer, read.start > 0, read.began),
        };
        // Keep a few views' worth: older ones only matter after "show earlier".
        let keep = VIEW_MESSAGES * depth * 4;
        let trimmed = messages.len() > keep;
        let messages: Vec<Value> = messages[messages.len().saturating_sub(keep)..].to_vec();
        let earlier = earlier || trimmed;
        let thread = &read.thread;
        self.view_cache = Some(ViewCache {
            env: env.clone(),
            id: id.clone(),
            depth,
            item_count: thread["itemCount"].as_u64().unwrap_or(0) as usize,
            status: str_of(&thread["status"]).to_string(),
            read_at,
            messages: messages.clone(),
            earlier,
        });
        let handoffs = self.handoffs(&env, &messages, &read.runs);
        let want = VIEW_MESSAGES * depth;
        let mut items: Vec<Value> = Vec::new();
        let mut positions: Vec<u64> = Vec::new();
        let mut embedded: Vec<Picture> = Vec::new();
        let mut chars = 0;
        for item in messages.iter().rev().take(want) {
            let kind = str_of(&item["type"]);
            if kind != "user_message" && kind != "assistant_message" {
                continue;
            }
            let truncated = item["textTruncated"] == true;
            let text = wow_text(str_of(&item["text"]), 6000) + if truncated { "\n…" } else { "" };
            let mut length = text.encode_utf16().count();
            let mut shown = json!({ "r": if kind == "user_message" { "u" } else { "a" }, "x": text });
            // A message with code or a table also comes cut into segments, which this addon
            // draws in its monospace font. Older addons read only the text, as does copying.
            let mut segments = wow_segments(str_of(&item["text"]), 6000);
            if !segments.is_empty() {
                if truncated {
                    segments.push(json!("…"));
                }
                length += segments.iter().map(|s| s.to_string().encode_utf16().count()).sum::<usize>();
                shown["g"] = json!(segments);
            }
            if let Some(line) = item["position"].as_u64().and_then(|p| handoffs.get(&p)) {
                shown["ho"] = json!(line);
            }
            // A message that started a fresh session (T3's /clear) gets T3's New session line
            // above it. T3's read doesn't say so yet, so it's the ones the game sent fresh.
            let message_id = str_of(&item["messageId"]);
            if item["freshContext"] == true || (!message_id.is_empty() && self.link.fresh_messages.iter().any(|m| m == message_id)) {
                shown["ns"] = json!(1);
            }
            if chars + length > VIEW_CHARS && !items.is_empty() {
                break;
            }
            chars += length;
            items.insert(0, shown);
            let position = item["position"].as_u64().unwrap_or(0);
            positions.insert(0, position);
            let role = if kind == "user_message" { "u" } else { "a" };
            for (alt, path) in embedded_files(str_of(&item["text"])).into_iter().rev() {
                if let Some(key) = file_key(&path) {
                    let label = if alt.is_empty() { "Image".to_string() } else { format!("Image: {alt}") };
                    embedded.insert(0, Picture { key, source: Source::File { path }, label, order: position * 2 + 1, role });
                }
            }
            if kind == "user_message" {
                for key in self.sent_pictures(&env, &id, item).into_iter().rev() {
                    embedded.insert(0, Picture { key, source: Source::Sent, label: "Pasted image".to_string(), order: position * 2 + 1, role });
                }
            }
        }
        let more = messages.len() > items.len() || earlier;
        let pending = thread["pendingRequestCount"].as_u64().unwrap_or(0) > 0;
        let status = if pending && working(str_of(&thread["status"])) { "waiting" } else { str_of(&thread["status"]) };
        let mut built = json!({
            "e": env,
            "id": id,
            "t": plain(str_of(&thread["title"]), 120),
            "s": status,
            "set": thread["settled"] == true,
            "at": iso_seconds(str_of(&thread["updatedAt"])),
            "more": more,
            "items": items,
        });
        // The branch its checkout is on, for the header. Only a thread read has it, not T3's
        // list, so the game shows it for the open thread alone.
        let branch = plain(str_of(&thread["branch"]), 80);
        if !branch.is_empty() {
            built["br"] = json!(branch);
        }
        // Its project, for that project's saved scripts (`snapshot` swaps it for them).
        if let Some(project) = thread["projectId"].as_str() {
            built["pid"] = json!(project);
        }
        // What it runs on and with what permissions, for the selectors over the box.
        if let Some(cfg) = thread_settings(thread, read.config.as_ref()) {
            built["cfg"] = cfg;
        }
        read.built = Some(built);
        read.positions = positions;
        read.embedded = embedded;
        if let Some(run) = self.poll_run.as_mut() {
            run.reading = Some(read);
        }
        if self.scans() {
            self.push_steps(vec![Step::ViewScan]);
            return;
        }
        self.name_links();
        if pending {
            self.push_steps(vec![Step::ViewAsk]);
        } else {
            self.finish_view(None);
        }
    }

    /// The view and its pictures are built: its question is read next, if it has one.
    fn ask_or_finish(&mut self) {
        let pending = self.poll_run.as_ref().and_then(|run| run.reading.as_ref()).is_some_and(|r| r.thread["pendingRequestCount"].as_u64().unwrap_or(0) > 0);
        if pending {
            self.push_steps(vec![Step::ViewAsk]);
        } else {
            self.finish_view(None);
        }
    }

    /// A page of the open thread's activity, looking for the pages an agent showed
    /// (html_render) from the first message shown on. What was read stays for the next
    /// read of the same thread, which goes on from there; a page still being shown is
    /// waited for. Then the pictures are planned.
    fn read_view_scan(&mut self) -> Result<(), T3Error> {
        let Some(read) = self.view_read() else { return Ok(()) };
        let Some(&first) = read.positions.first() else {
            self.plan_pictures();
            return Ok(());
        };
        let View { env, id, .. } = read.view.clone();
        let mut cache = match self.scan.take().filter(|c| c.env == env && c.id == id && c.from <= first) {
            Some(cache) => cache,
            None => ScanCache { env: env.clone(), id: id.clone(), from: first, upto: None, pages: Vec::new(), cards: Vec::new(), compactions: Vec::new(), changes: Vec::new(), work: Vec::new(), runs: BTreeMap::new() },
        };
        let mut args = json!({ "threadId": id, "view": "activity", "limit": 100, "maxCharsPerItem": 200 });
        // Back to what was still under way, whose title and text may have come since.
        let unfinished = cache.work.iter().filter(|w| !w.ended).map(|w| w.position).min().and_then(|p| p.checked_sub(1));
        if let Some(after) = [cache.upto.or(first.checked_sub(1)), unfinished].into_iter().flatten().min() {
            args["afterPosition"] = json!(after);
        }
        let result = match self.client.call("t3_thread_read", with_args(args, &self.env_arg(&env))) {
            Ok(result) => result,
            Err(error) => {
                self.scan = Some(cache);
                return Err(error);
            }
        };
        let now = self.now_ms();
        let mut waiting = false;
        for item in result["items"].as_array().into_iter().flatten() {
            let Some(position) = item["position"].as_u64() else { continue };
            if let Some(run) = item["runId"].as_str() {
                cache.runs.insert(position, (run.to_string(), iso_seconds(str_of(&item["updatedAt"]))));
            }
            if let Some(mut work) = FoundWork::from_item(item, position, now) {
                match cache.work.iter_mut().find(|w| w.position == position) {
                    Some(known) => {
                        work.answer = known.answer.take();
                        work.answered = known.answered;
                        *known = work;
                    }
                    None => cache.work.push(work),
                }
            }
            if item["type"] == "subagent" {
                let card = FoundCard::from_item(item, position);
                match cache.cards.iter_mut().find(|c| c.item_id == card.item_id) {
                    Some(known) if known.full && known.status == card.status => {}
                    Some(known) => *known = card,
                    None if !card.item_id.is_empty() => cache.cards.push(card),
                    None => {}
                }
            }
            // A turn that changed no files has an empty list ("[]"): nothing to show.
            if item["type"] == "checkpoint" {
                let item_id = str_of(&item["itemId"]);
                let text = str_of(&item["text"]).trim();
                if !item_id.is_empty() && text.starts_with("[\n") && !cache.changes.iter().any(|c| c.item_id == item_id) {
                    let cut = item["textTruncated"] == true;
                    cache.changes.push(FoundChanges { position, item_id: item_id.to_string(), files: changed_files(text, cut), cut, full: !cut });
                }
            }
            let compaction = item["type"] == "compaction";
            if compaction || (item["type"] == "dynamic_tool" && is_page_item(str_of(&item["text"]))) {
                match str_of(&item["status"]) {
                    "completed" if compaction => {
                        if !cache.compactions.contains(&position) {
                            cache.compactions.push(position);
                        }
                    }
                    "completed" => {
                        let item_id = str_of(&item["itemId"]).to_string();
                        if !item_id.is_empty() && !cache.pages.iter().any(|p| p.item_id == item_id) {
                            cache.pages.push(FoundPage { position, item_id, title: page_title(str_of(&item["text"])) });
                        }
                    }
                    "failed" | "declined" | "cancelled" | "interrupted" | "error" => {}
                    // Still under way (unless it has been at it for ages): look again next time.
                    _ if now - iso_seconds(str_of(&item["updatedAt"])) * 1000 < 600_000 => {
                        waiting = true;
                        break;
                    }
                    _ => {}
                }
            }
            cache.upto = Some(position);
        }
        let more = result["hasMore"] == true && !waiting;
        self.scan = Some(cache);
        let Some(read) = self.view_read() else { return Ok(()) };
        read.scan_pages += 1;
        if more && read.scan_pages < SCAN_PAGES {
            self.push_steps(vec![Step::ViewScan]);
        } else {
            read.again |= more || waiting;
            self.plan_pictures();
        }
        Ok(())
    }

    /// The pictures the view shows: the pages found from its first message on, and the
    /// images its messages embed. Those not in an image slot yet are drawn now, a few a
    /// poll; the rest wait for the next one.
    fn plan_pictures(&mut self) {
        let pictures_ok = self.images_ok && self.painter.is_some();
        let found = self.scan.as_ref().filter(|_| pictures_ok).map(|c| c.pages.clone()).unwrap_or_default();
        let late = self.out_of_time();
        let now = self.now_ms();
        let Some(read) = self.view_read() else { return };
        let View { env, id, .. } = read.view.clone();
        let first = read.positions.first().copied().unwrap_or(u64::MAX);
        let mut pictures: Vec<Picture> = found
            .into_iter()
            .filter(|p| p.position >= first)
            .map(|p| Picture {
                key: format!("page:{env}:{id}:{}:{DRAWING_VERSION}", p.item_id),
                label: if p.title.is_empty() { "Page".into() } else { format!("Page: {}", p.title) },
                source: Source::Page { item_id: p.item_id },
                order: p.position * 2,
                role: "a",
            })
            .collect();
        if pictures_ok {
            pictures.extend(read.embedded.iter().cloned());
        }
        // Delegated tasks to read again: still running, or ended with more result than
        // the scan read (once).
        let cards: Vec<String> = match (self.cards_ok, self.scan.as_ref()) {
            (true, Some(scan)) => scan.cards.iter().filter(|c| c.position >= first && (!c.ended() || (c.truncated && !c.full))).map(|c| c.item_id.clone()).collect(),
            _ => Vec::new(),
        };
        let reads = if late { 0 } else { cards.len().min(CARD_READS_PER_POLL) };
        // Questions whose answer hasn't been looked for yet.
        let answers: Vec<u64> = match (self.work_ok, self.scan.as_ref()) {
            (true, Some(scan)) => scan.work.iter().filter(|w| w.kind == WorkKind::Question && w.ended && !w.answered && w.position >= first).map(|w| w.position).collect(),
            _ => Vec::new(),
        };
        let answer_reads = if late { 0 } else { answers.len().min(CARD_READS_PER_POLL) };
        // Changed-file lists longer than the scan read, read whole once.
        let changes: Vec<String> = match (self.work_ok, self.scan.as_ref()) {
            (true, Some(scan)) => scan.changes.iter().filter(|c| !c.full && c.position >= first).map(|c| c.item_id.clone()).collect(),
            _ => Vec::new(),
        };
        let change_reads = if late { 0 } else { changes.len().min(CARD_READS_PER_POLL) };
        let mut drawing: Vec<Drawing> = Vec::new();
        let mut later = false;
        for picture in &pictures {
            // A page or a file emptied since it was drawn (a reinstall) is drawn again.
            let held = self.link.images.iter().find(|(_, slot)| slot.key == picture.key).map(|(n, _)| *n);
            if let Some(n) = held.filter(|n| !matches!(picture.source, Source::Sent) && !self.slots.has_image(*n)) {
                self.link.images.remove(&n);
            }
            let stored = self.link.images.values().any(|slot| slot.key == picture.key);
            let failed = self.picture_errors.get(&picture.key).is_some_and(|(_, at)| now - at < PICTURE_RETRY_MS);
            if stored || failed || drawing.iter().any(|d| d.picture.key == picture.key) {
                continue;
            }
            if drawing.len() < DRAWS_PER_POLL && !late {
                drawing.push(Drawing { picture: picture.clone(), text: String::new(), offset: None, pieces: 0 });
            } else {
                later = true;
            }
        }
        let Some(read) = self.view_read() else { return };
        read.pictures = pictures;
        read.again |= later || reads < cards.len() || answer_reads < answers.len() || change_reads < changes.len();
        let steps: Vec<Step> = cards
            .into_iter()
            .take(reads)
            .map(Step::ViewCard)
            .chain(answers.into_iter().take(answer_reads).map(Step::ViewAnswer))
            .chain(changes.into_iter().take(change_reads).map(Step::ViewChanges))
            .chain((0..drawing.len()).map(Step::ViewDraw))
            .chain([Step::ViewPictures])
            .collect();
        read.drawing = drawing;
        self.push_steps(steps);
    }

    /// A delegated task's item read again, for its status and (once it ended) CARD_CHARS of
    /// its result. A task that ends adds to its thread (T3 tells the agent), so the view is
    /// read again then, and this with it.
    fn read_view_card(&mut self, item_id: String) -> Result<(), T3Error> {
        let Some(read) = self.view_read() else { return Ok(()) };
        let View { env, id, .. } = read.view.clone();
        let Some(position) = self.scan.as_ref().and_then(|s| s.cards.iter().find(|c| c.item_id == item_id)).map(|c| c.position) else { return Ok(()) };
        let mut args = json!({ "threadId": id, "view": "activity", "limit": 1, "maxCharsPerItem": CARD_CHARS });
        if let Some(after) = position.checked_sub(1) {
            args["afterPosition"] = json!(after);
        }
        let result = self.client.call("t3_thread_read", with_args(args, &self.env_arg(&env)))?;
        let item = result["items"].as_array().and_then(|items| items.iter().find(|it| it["itemId"] == item_id.as_str())).cloned();
        if let (Some(item), Some(scan)) = (item, self.scan.as_mut()) {
            if let Some(card) = scan.cards.iter_mut().find(|c| c.item_id == item_id) {
                *card = FoundCard::from_item(&item, position);
                card.full = card.ended();
            }
        }
        Ok(())
    }

    /// What you answered the question at `position`: its AskUserQuestion call comes just
    /// before it. Read once; a provider with no such call shows the question alone.
    fn read_view_answer(&mut self, position: u64) -> Result<(), T3Error> {
        let Some(read) = self.view_read() else { return Ok(()) };
        let View { env, id, .. } = read.view.clone();
        let mut args = json!({ "threadId": id, "view": "activity", "limit": 2, "maxCharsPerItem": DETAIL_CHARS });
        if let Some(after) = position.checked_sub(2) {
            args["afterPosition"] = json!(after);
        }
        let result = self.client.call("t3_thread_read", with_args(args, &self.env_arg(&env)))?;
        let call = result["items"].as_array().into_iter().flatten().filter_map(|item| serde_json::from_str::<Value>(str_of(&item["text"])).ok()).find(|call| call["toolName"] == "AskUserQuestion");
        let answer = call.and_then(|call| call["output"]["answers"].as_object().map(|answers| answers.values().filter_map(Value::as_str).map(|a| plain(a, 200)).collect::<Vec<_>>().join(" · ")));
        if let Some(found) = self.scan.as_mut().and_then(|scan| scan.work.iter_mut().find(|w| w.position == position)) {
            found.answer = answer.filter(|a| !a.is_empty());
            found.answered = true;
        }
        Ok(())
    }

    /// The files a turn changed, read whole (up to CHANGES_CHARS) from its checkpoint item,
    /// once. A list longer than that shows the files it holds, and says there are more.
    fn read_view_changes(&mut self, item_id: String) -> Result<(), T3Error> {
        let Some(read) = self.view_read() else { return Ok(()) };
        let View { env, id, .. } = read.view.clone();
        let args = json!({ "threadId": id, "itemId": item_id, "maxCharsPerItem": CHANGES_CHARS });
        let result = self.client.call("t3_thread_read", with_args(args, &self.env_arg(&env)))?;
        let item = result["items"].as_array().and_then(|items| items.iter().find(|it| it["itemId"] == item_id.as_str()));
        if let Some(found) = self.scan.as_mut().and_then(|scan| scan.changes.iter_mut().find(|c| c.item_id == item_id)) {
            if let Some(item) = item {
                found.cut = item["textTruncated"] == true;
                found.files = changed_files(str_of(&item["text"]), found.cut);
            }
            found.full = true;
        }
        Ok(())
    }

    /// Draws the `i`th picture of this poll into an image slot. A page's HTML is read
    /// first, a request (a piece of its item) per step.
    fn draw_picture(&mut self, i: usize) -> Result<(), T3Error> {
        let Some(read) = self.view_read() else { return Ok(()) };
        let View { env, id, .. } = read.view.clone();
        let Some(job) = read.drawing.get(i) else { return Ok(()) };
        let picture = job.picture.clone();
        let offset = job.offset;
        let drawn = match &picture.source {
            Source::Page { item_id } => {
                let mut args = json!({ "threadId": id, "itemId": item_id, "maxCharsPerItem": 50_000 });
                if let Some(offset) = offset {
                    args["textOffset"] = json!(offset);
                }
                let result = self.client.call("t3_thread_read", with_args(args, &self.env_arg(&env)))?;
                let item = result["items"].as_array().and_then(|items| items.iter().find(|it| it["itemId"] == item_id.as_str())).cloned().unwrap_or(Value::Null);
                let Some(read) = self.view_read() else { return Ok(()) };
                let Some(job) = read.drawing.get_mut(i) else { return Ok(()) };
                job.text.push_str(str_of(&item["text"]));
                job.pieces += 1;
                match item["nextTextOffset"].as_u64().filter(|_| item["textTruncated"] == true) {
                    Some(next) if job.pieces < PAGE_TEXT_PIECES => {
                        job.offset = Some(next);
                        self.push_steps(vec![Step::ViewDraw(i)]);
                        return Ok(());
                    }
                    Some(_) => Err("the page is too large".to_string()),
                    None => {
                        let text = std::mem::take(&mut job.text);
                        self.draw_page(&text, &env)
                    }
                }
            }
            Source::File { path } => match self.painter.as_ref() {
                Some(painter) => painter.picture(path),
                None => Err("no browser to draw with".into()),
            },
            Source::Sent => Err("its picture is gone".into()),
        };
        let stored = drawn.and_then(|bitmap| self.store_picture(&picture.key, &bitmap));
        if let Err(error) = stored {
            self.log(&format!("couldn't draw {} for the game: {error}", picture.label));
            let at = self.now_ms();
            self.picture_errors.insert(picture.key.clone(), (error, at));
        }
        Ok(())
    }

    /// Draws a page from its html_render item's text (the tool call, as JSON), in a thread
    /// on machine `env`.
    fn draw_page(&self, text: &str, env: &str) -> Result<Bitmap, String> {
        // T3 keeps a page's snapshot next to its settings, on the machine the thread runs on.
        let attachments = self.t3_settings.as_ref().filter(|_| self.env_arg(env).is_empty()).and_then(|f| f.parent()).map(|d| d.join("attachments"));
        let html = page_html(text, attachments.as_deref())?;
        let height = serde_json::from_str::<Value>(text).ok().and_then(|call| call["input"]["height"].as_u64()).unwrap_or(400);
        self.painter.as_ref().ok_or("no browser to draw with")?.page(&html, page_height(height))
    }

    /// Puts a drawn picture in an image slot the game process hasn't been told about: an
    /// empty one first, else the one shown longest ago.
    fn store_picture(&mut self, key: &str, bitmap: &Bitmap) -> Result<(), String> {
        let shown: HashSet<u32> = self.link.images_shown.iter().copied().collect();
        let empty = (1..=IMAGE_COUNT).find(|n| !shown.contains(n) && !self.link.images.contains_key(n));
        let oldest = || self.link.images.iter().filter(|(n, _)| !shown.contains(n)).min_by_key(|(_, slot)| slot.used).map(|(n, _)| *n);
        let Some(n) = empty.or_else(oldest) else { return Err(IMAGES_OUT.into()) };
        // Out of the map first: a slot half written holds no picture.
        self.link.images.remove(&n);
        self.slots.write_image(n, &tga(bitmap)).map_err(|e| format!("couldn't write the picture: {e}"))?;
        self.link.images.insert(n, ImageSlot { key: key.to_string(), w: bitmap.width, h: bitmap.height, used: 0 });
        self.save_state();
        Ok(())
    }

    /// The view with its pictures among its messages: each one in an image slot shows as a
    /// picture, and is named to the game process (so the slot keeps it until WoW quits);
    /// one not drawn yet, or that couldn't be, shows as a line saying so.
    fn place_pictures(&mut self) {
        let now = self.now_ms();
        let Some(read) = self.view_read() else { return };
        let pictures = std::mem::take(&mut read.pictures);
        read.drawing.clear();
        let mut again = false;
        let mut placed: Vec<(u64, Value)> = Vec::new();
        for picture in pictures {
            let slot = self.link.images.iter().find(|(_, slot)| slot.key == picture.key).map(|(n, slot)| (*n, slot.w, slot.h));
            let label = plain(&picture.label, 120);
            let item = match slot {
                // Its file was emptied (a reinstall): naming it would show the game's
                // placeholder for a blank file until WoW quits.
                Some((n, _, _)) if !self.slots.has_image(n) => {
                    json!({ "r": picture.role, "x": format!("|cff808080[{label}] (couldn't be loaded)|r"), "lost": 1 })
                }
                Some((n, w, h)) => {
                    if let Some(slot) = self.link.images.get_mut(&n) {
                        slot.used = now;
                    }
                    if !self.link.images_shown.contains(&n) {
                        self.link.images_shown.push(n);
                    }
                    let mut im = json!({ "f": n, "w": w, "h": h });
                    // An image pasted in the game stays small there, until it's clicked.
                    if matches!(picture.source, Source::Sent) {
                        im["sm"] = json!(1);
                    }
                    json!({ "r": picture.role, "x": format!("[{label}]"), "im": im })
                }
                None => {
                    let why = match self.picture_errors.get(&picture.key) {
                        Some((error, _)) => format!("couldn't show it in the game: {}", plain(error, 160)),
                        None => {
                            again = true;
                            "drawing it for the game…".to_string()
                        }
                    };
                    json!({ "r": picture.role, "x": format!("|cff808080[{label}] ({why})|r") })
                }
            };
            placed.push((picture.order, item));
        }
        if !placed.is_empty() {
            self.save_state();
        }
        if self.cards_ok {
            let first = self.view_read().and_then(|r| r.positions.first().copied()).unwrap_or(u64::MAX);
            for card in self.scan.iter().flat_map(|s| s.cards.iter()).filter(|c| c.position >= first) {
                placed.push((card.position * 2, card.shown()));
            }
            for position in self.scan.iter().flat_map(|s| s.compactions.iter()).filter(|p| **p >= first) {
                placed.push((position * 2, json!({ "r": "a", "x": "Context compacted", "rule": 1 })));
            }
        }
        if self.work_ok {
            let first = self.view_read().and_then(|r| r.positions.first().copied()).unwrap_or(u64::MAX);
            // Each finished turn's changed files, where T3 shows them: after its last message.
            for changes in self.scan.iter().flat_map(|s| s.changes.iter()).filter(|c| c.full && c.position >= first && !c.files.is_empty()) {
                placed.push((changes.position * 2, changes.shown()));
            }
            let Some(read) = self.view_read() else { return };
            let still_working = read.built.as_ref().is_some_and(|built| working(str_of(&built["s"])));
            let done_before = still_working.then(|| read.positions.last().copied().unwrap_or(0));
            let walls: Vec<u64> = read.positions.iter().copied().chain(placed.iter().map(|(order, _)| order / 2)).collect();
            let rows = self.scan.as_ref().map(|scan| work_rows(&scan.work, first, done_before, &walls)).unwrap_or_default();
            placed.extend(rows);
        }
        let work_ok = self.work_ok;
        let run_at = self.scan.as_ref().filter(|_| work_ok).map(|scan| scan.runs.clone()).unwrap_or_default();
        let Some(read) = self.view_read() else { return };
        read.again |= again;
        if let Some(built) = read.built.as_mut().filter(|_| !placed.is_empty() || work_ok) {
            let items = built["items"].as_array().cloned().unwrap_or_default();
            let mut all: Vec<(u64, Value)> = read.positions.iter().map(|p| p * 2).zip(items).collect();
            all.extend(placed);
            all.sort_by_key(|(order, _)| *order);
            // A message that is only its images is just the pictures, with no empty bubble.
            let pictures: HashSet<u64> = all.iter().filter(|(_, item)| item["im"].is_object() || item["lost"] == 1).map(|(order, _)| *order).collect();
            all.retain(|(order, item)| !(item["r"] == "u" && str_of(&item["x"]).trim().is_empty() && pictures.contains(&(order + 1))));
            if work_ok {
                fold_turns(&mut all, &read.positions, &run_at, &read.runs);
            }
            built["items"] = json!(all.into_iter().map(|(_, item)| item).collect::<Vec<_>>());
        }
        self.name_links();
        self.ask_or_finish();
    }

    /// Whether the open thread's activity is read: for pictures, or for delegated tasks'
    /// cards. An addon that shows neither needs only the messages.
    fn scans(&self) -> bool {
        self.cards_ok || self.work_ok || (self.images_ok && self.painter.is_some())
    }

    /// Reads what a row did, in one request, for the update that answers the click. A
    /// failure is shown where the row's contents would be, and can be tried again.
    fn open_detail(&mut self, env: &str, id: &str, first: u64, last: u64) -> Result<(), T3Error> {
        let limit = (last.saturating_sub(first) + 1).min(WORK_ROW_SPAN);
        let mut args = json!({ "threadId": id, "view": "activity", "limit": limit, "maxCharsPerItem": DETAIL_CHARS });
        if let Some(after) = first.checked_sub(1) {
            args["afterPosition"] = json!(after);
        }
        let (mut body, failed) = match self.client.call("t3_thread_read", with_args(args, &self.env_arg(env))) {
            Ok(result) => (detail_segments(result["items"].as_array().map(Vec::as_slice).unwrap_or_default()), false),
            Err(T3Error::Auth(message)) => return Err(T3Error::Auth(message)),
            Err(T3Error::Other(error)) => {
                self.log(&format!("couldn't read a row of the thread: {error}"));
                (vec![json!(format!("Couldn't read this from T3: {}", plain(&error, 160)))], true)
            }
        };
        // Its prose names models as the conversation does (code comes as { c = code }).
        let providers = self.catalogs.get(env).map_or(&[][..], |c| c.providers.as_slice());
        for segment in &mut body {
            if let Value::String(text) = segment {
                *text = name_model_refs(text, |instance, model| model_ref_name(providers, instance, model, false));
            }
        }
        self.details.retain(|d| !(d.env == env && d.id == id && d.last == last));
        self.details.push(Detail { env: env.to_string(), id: id.to_string(), last, body, failed });
        let excess = self.details.len().saturating_sub(DETAILS_KEPT);
        self.details.drain(..excess);
        Ok(())
    }

    /// The thread links in the built view, named: a delegated task's or a listed thread's
    /// title in quotes, else what it is ("a delegated task", "a thread").
    fn name_links(&mut self) {
        let cards: HashMap<String, String> = self
            .scan
            .iter()
            .flat_map(|s| s.cards.iter())
            .filter(|c| !c.title.trim().is_empty())
            .map(|c| (c.task().to_string(), c.title.clone()))
            .collect();
        let threads = &self.threads;
        let Some(read) = self.poll_run.as_mut().and_then(|run| run.reading.as_mut()) else { return };
        let Some(items) = read.built.as_mut().and_then(|b| b["items"].as_array_mut()) else { return };
        let name = |env: &str, thread: &str| {
            let card = cards.get(task_part(thread)).filter(|_| thread.starts_with("thread:")).cloned();
            let listed = || threads.get(env).and_then(|list| list.iter().find(|t| t["threadId"] == thread)).map(|t| str_of(&t["title"]).to_string());
            match card.or_else(listed).filter(|t| !t.trim().is_empty()) {
                Some(title) => format!("“{}”", plain(&title, 120)),
                None if thread.contains("delegated-task:") => "a delegated task".to_string(),
                None => "a thread".to_string(),
            }
        };
        let providers = self.catalogs.get(&read.view.env).map_or(&[][..], |c| c.providers.as_slice());
        let model = |cell: bool| move |instance: &str, model: &str| model_ref_name(providers, instance, model, cell);
        let (prose, cells) = (model(false), model(true));
        // A table's rows ({ t = rows }) are plain text, the rest game markup.
        fn walk(value: &mut Value, name: &dyn Fn(&str, &str) -> String, model: &dyn Fn(&str, &str) -> Option<String>, cells: &dyn Fn(&str, &str) -> Option<String>) {
            match value {
                Value::String(text) => *text = name_model_refs(&name_thread_links(text, name), model),
                Value::Array(list) => list.iter_mut().for_each(|v| walk(v, name, model, cells)),
                Value::Object(map) => {
                    for (key, v) in map.iter_mut() {
                        let table = key == "t" && v.is_array();
                        walk(v, name, if table { cells } else { model }, cells);
                    }
                }
                _ => {}
            }
        }
        for item in items {
            // Copying keeps each model T3 names as T3 has it (model:...), which the view
            // shows by name.
            let marked = item["x"].as_str().filter(|x| x.contains(MODEL_MARK)).map(str::to_string);
            walk(item, &name, &prose, &cells);
            if let Some(text) = marked {
                let copy = name_thread_links(&name_model_refs(&text, |_, _| None), name);
                if item["x"] != copy.as_str() {
                    item["cx"] = json!(copy);
                }
            }
        }
    }

    /// The question the open thread waits on: which request it is (it's read next).
    fn read_view_ask(&mut self) -> Result<(), T3Error> {
        let Some(read) = self.view_read() else { return Ok(()) };
        let View { env, id, .. } = read.view.clone();
        let args = with_args(json!({ "action": "list", "threadId": id }), &self.env_arg(&env));
        match self.client.call("t3_pending_request", args) {
            Ok(list) => match list["requestIds"].as_array().and_then(|ids| ids.first()).and_then(Value::as_str) {
                Some(request_id) => self.push_steps(vec![Step::ViewAskRead(request_id.to_string())]),
                None => self.finish_view(Some(Ok(None))),
            },
            Err(error) => self.finish_view(Some(Err(error))),
        }
        Ok(())
    }

    /// The question the open thread waits on, read; the view is then done.
    fn read_view_request(&mut self, request_id: String) -> Result<(), T3Error> {
        let Some(read) = self.view_read() else { return Ok(()) };
        let View { env, id, .. } = read.view.clone();
        let args = with_args(json!({ "action": "read", "threadId": id, "requestId": request_id }), &self.env_arg(&env));
        let ask = self.client.call("t3_pending_request", args).map(|request| Self::pending_ask(&env, &id, &request_id, &request));
        self.finish_view(Some(ask));
        Ok(())
    }

    /// The open thread's view goes to the game, with its question (`ask`: None when nothing
    /// is pending). A question the agent is waiting on goes with the conversation.
    /// Approvals can't be answered through T3's tools: the window points to T3 instead.
    /// If the question can't be read this time, the one shown stays.
    fn finish_view(&mut self, ask: Option<Result<Option<(PendingAsk, Value)>, T3Error>>) {
        let Some(read) = self.poll_run.as_mut().and_then(|run| run.reading.take()) else { return };
        let View { env, id, .. } = read.view;
        let thread = read.thread;
        let Some(mut view) = read.built else { return };
        let pending = thread["pendingRequestCount"].as_u64().unwrap_or(0) > 0;
        let shown_ask = self.view_data.as_ref().filter(|v| v["e"] == env.as_str() && v["id"] == id.as_str()).map(|v| v["ask"].clone()).filter(|a| !a.is_null());
        let previous = self.pending_ask.take().filter(|a| a.env == env && a.thread == id);
        match ask {
            Some(Ok(Some((ask, value)))) => {
                view["ask"] = value;
                self.pending_ask = Some(ask);
            }
            Some(Ok(None)) | None => {}
            Some(Err(error)) => {
                self.log(&format!("pending question: {error}"));
                if let (Some(ask), Some(value)) = (previous, shown_ask) {
                    view["ask"] = value;
                    self.pending_ask = Some(ask);
                }
            }
        }
        // Waiting on something that isn't a question: an approval, which only T3 can give.
        if self.pending_ask.is_none() && pending {
            view["approval"] = json!(true);
            if working(str_of(&thread["status"])) {
                view["s"] = json!("approval");
            }
        }
        // Waiting on a card only T3 can answer (a secret, a machine link): the window names
        // it and points to T3. The game never takes the answer: what goes between it and the
        // companion sits in files on disk.
        let card = self.pending_counts.get(&format!("{env}:{id}")).and_then(|p| p.card.clone());
        if let Some(card) = card.filter(|_| !pending && working(str_of(&thread["status"]))) {
            view["card"] = json!({ "k": card.kind, "l": card.label });
            view["s"] = json!(CARD);
        }
        self.view_data = Some(view);
        // Pictures not ready yet: read the view again next poll.
        self.view_key = (!read.again).then_some(read.key);
    }

    /// The first question of a pending request, for the game: a short token, and each
    /// question with its options. The full ids stay here; the game answers by index.
    fn pending_ask(env: &str, thread: &str, request_id: &str, request: &Value) -> Option<(PendingAsk, Value)> {
        let mut questions = Vec::new();
        let mut shown = Vec::new();
        for q in request["questions"].as_array().into_iter().flatten() {
            let text = str_of(&q["question"]);
            let key = q["id"].as_str().unwrap_or(text).to_string();
            let options: Vec<&Value> = q["options"].as_array().map(|o| o.iter().collect()).unwrap_or_default();
            let multi = q["multiSelect"] == true;
            questions.push(PendingQuestion { key, labels: options.iter().map(|o| str_of(&o["label"]).to_string()).collect(), multi });
            shown.push(json!({
                "h": plain(str_of(&q["header"]), 40),
                // Not in the view, which names models: a model reference stays as written.
                "q": name_model_refs(&wow_text(text, 1000), |_, _| None),
                "m": multi,
                "o": options.iter().map(|o| json!({ "l": plain(str_of(&o["label"]), 60), "d": plain(str_of(&o["description"]), 160) })).collect::<Vec<_>>(),
            }));
        }
        if questions.is_empty() {
            return None;
        }
        let token = format!("{:08x}", hash_of(&json!(request_id)) as u32);
        let ask = PendingAsk { env: env.to_string(), thread: thread.to_string(), token: token.clone(), request_id: request_id.to_string(), questions };
        Some((ask, json!({ "k": token, "qs": shown })))
    }

    /// Turns the game's answer (per question: a 1-based option index, a list of them, or
    /// typed text) into T3's answers, keyed by question id with option labels. A list may
    /// end with typed text, a note on the picks: T3's answers have no place for one (its own
    /// window sends either), so it goes in the answer after them, as "Note: ...".
    fn answers_for(ask: &PendingAsk, given: &Value) -> Map<String, Value> {
        let mut answers = Map::new();
        for (i, q) in ask.questions.iter().enumerate() {
            let pick = |v: &Value| v.as_u64().and_then(|n| q.labels.get((n as usize).wrapping_sub(1))).cloned();
            let value = match &given[i] {
                Value::String(text) if !text.trim().is_empty() => Some(json!(text.trim())),
                Value::Number(_) if q.multi => pick(&given[i]).map(|l| json!([l])),
                Value::Number(_) => pick(&given[i]).map(Value::String),
                Value::Array(list) => {
                    let mut labels: Vec<String> = list.iter().filter_map(pick).collect();
                    let note = list.iter().filter_map(Value::as_str).map(str::trim).find(|t| !t.is_empty());
                    match (labels.is_empty(), note) {
                        (true, None) => None,
                        (true, Some(note)) => Some(json!(note)),
                        (false, Some(note)) if q.multi => {
                            labels.push(format!("Note: {note}"));
                            Some(json!(labels))
                        }
                        (false, Some(note)) => {
                            let label = &labels[0];
                            let stop = if label.ends_with(['.', '!', '?']) { "" } else { "." };
                            Some(json!(format!("{label}{stop} Note: {note}")))
                        }
                        (false, None) if q.multi => Some(json!(labels)),
                        (false, None) => Some(json!(labels[0])),
                    }
                }
                _ => None,
            };
            if let Some(value) = value {
                answers.insert(q.key.clone(), value);
            }
        }
        answers
    }

    // ---- writing into the game ----

    pub fn snapshot(&self) -> Value {
        // 1-based, as the game indexes inbox.envs; 0 (left out) for a machine no longer linked.
        let env_index = |id: &str| self.envs.iter().position(|e| e.id == id).map_or(0, |i| i + 1);
        let mut project_title: HashMap<String, String> = HashMap::new();
        for (env, list) in &self.projects {
            for p in list {
                project_title.insert(format!("{env}:{}", str_of(&p["id"])), str_of(&p["title"]).to_string());
            }
        }
        let mut threads: Vec<Value> = Vec::new();
        for (env, list) in self.threads.iter().filter(|(env, _)| env_index(env) > 0) {
            for t in list {
                let mut entry = json!({
                    "e": env_index(env),
                    "id": str_of(&t["threadId"]),
                    "t": plain(str_of(&t["title"]), 90),
                    "s": str_of(&t["status"]),
                    "set": t["settled"] == true,
                });
                if t["snoozed"] == true {
                    entry["sn"] = json!(true);
                }
                // An agent made it: taken for a task delegated from another machine (or not
                // read yet), the game keeps it out of Open while it's idle, as T3's sidebar
                // does. One T3 lists has a row of its own (`find_launcher`), and one started
                // from the game is yours, whatever T3 recorded.
                if t["createdBy"] == "agent" && t["ownRow"] != true && !self.link.started_threads.iter().any(|known| known == str_of(&t["threadId"])) {
                    entry["ag"] = json!(true);
                }
                if let Some(started) = t["runStartedAt"].as_i64() {
                    entry["st"] = json!(started);
                }
                if let Some(launcher) = t["launcher"].as_str() {
                    entry["par"] = json!(launcher);
                }
                if let Some((label, state)) = pull_request_label(t) {
                    entry["pr"] = json!(label);
                    entry["ps"] = json!(state);
                    entry["prs"] = json!(pull_request_links(t));
                }
                entry["at"] = json!(iso_seconds(str_of(&t["updatedAt"])));
                // The row's age, as T3 0.18's sidebar tells it: a settled thread's from when it
                // was settled, any other's from its last activity (`read_activity`).
                let settled_at = iso_seconds(str_of(&t["settledAt"]));
                if t["settled"] == true && settled_at > 0 {
                    entry["la"] = json!(settled_at);
                } else if let Some(at) = t["activityAt"].as_i64() {
                    entry["la"] = json!(at);
                }
                let project = project_title.get(&format!("{env}:{}", str_of(&t["projectId"]))).map(String::as_str).unwrap_or("");
                entry["p"] = json!(plain(project, 40));
                threads.push(entry);
            }
        }
        threads.sort_by_key(|t| std::cmp::Reverse(t["at"].as_i64().unwrap_or(0)));
        let mut projects = Vec::new();
        for (env, list) in self.projects.iter().filter(|(env, _)| env_index(env) > 0) {
            let machine_worktree = self.new_worktrees.get(env).copied().unwrap_or(false);
            for p in list.iter().filter(|p| !p["archivedAt"].is_string() && !p["deletedAt"].is_string() && !is_scratch_project(p)) {
                let mut entry = json!({ "e": env_index(env), "id": str_of(&p["id"]), "t": plain(str_of(&p["title"]), 40) });
                // Where New Thread starts it by default: the project's own mode, else the machine's.
                let worktree = match p["defaultThreadEnvMode"].as_str() {
                    Some(mode) => mode == "worktree",
                    None => machine_worktree,
                };
                if worktree {
                    entry["w"] = json!(true);
                }
                projects.push(entry);
            }
        }
        let view = self.view_data.as_ref().map(|data| {
            let mut data = data.clone();
            // From the list, read every poll: a merge needn't touch the thread, so the view
            // wouldn't be read again for it.
            let links = self.threads.get(str_of(&data["e"])).and_then(|list| list.iter().find(|t| t["threadId"] == data["id"])).map(pull_request_links);
            if let Some(links) = links.filter(|l| !l.is_empty()) {
                data["prs"] = json!(links);
            }
            // By name and id only: what a script runs stays out of the game.
            if let Some(project) = data.as_object_mut().and_then(|d| d.remove("pid")) {
                match self.project_scripts(str_of(&data["e"]), str_of(&project)) {
                    Some([]) => {}
                    Some(scripts) => data["scr"] = json!(scripts.iter().map(|s| json!({ "id": s["id"], "n": plain(str_of(&s["name"]), 40) })).collect::<Vec<_>>()),
                    None => data["sna"] = json!(true),
                }
            }
            let details: Vec<Value> = self
                .details
                .iter()
                .filter(|d| d.env == str_of(&data["e"]) && d.id == str_of(&data["id"]))
                .map(|d| if d.failed { json!({ "l": d.last, "b": d.body, "bad": 1 }) } else { json!({ "l": d.last, "b": d.body }) })
                .collect();
            if !details.is_empty() {
                data["det"] = json!(details);
            }
            data["e"] = json!(env_index(str_of(&data["e"])));
            data
        });
        let client_key = self.link.client_key.clone();
        let mut out = Map::new();
        out.insert("v".into(), json!(1));
        out.insert("k".into(), json!(client_key));
        // The UI session this is for: an older session's acks mean nothing to a newer one.
        out.insert("u".into(), json!(self.link.ui_session));
        // The T3 sign-in this is from: the game drops what it had from another one.
        out.insert("lid".into(), json!(self.link.login));
        out.insert("ack".into(), json!(client_key.as_ref().and_then(|k| self.link.acked.get(k)).copied().unwrap_or(0)));
        // This companion reads long commands in parts: the addon only draws them if it says so.
        out.insert("lm".into(), json!(1));
        let envs: Vec<Value> = self
            .envs
            .iter()
            .map(|e| {
                let mut entry = json!({ "id": e.id, "name": plain(&e.name, 40), "ok": e.ok });
                // The permissions a new thread on it gets when none is picked.
                if let Some(mode) = self.catalogs.get(&e.id).and_then(|c| c.mode.as_ref()) {
                    entry["dm"] = json!(mode);
                }
                // Its threads can start a fresh session (T3's /clear).
                if self.fresh_context_on(&e.id) {
                    entry["fc"] = json!(true);
                }
                entry
            })
            .collect();
        out.insert("envs".into(), json!(envs));
        out.insert("threads".into(), json!(threads));
        out.insert("settledTotal".into(), json!(self.settled_totals.values().sum::<u64>()));
        // The groups with more threads than listed that Show more can still raise (left
        // out when there are none).
        let mut more = Map::new();
        for (n, name) in LIST_NAMES.iter().enumerate() {
            let unlisted = self.list_more.iter().any(|(env, m)| m[n] && env_index(env) > 0);
            if unlisted && self.list_limits[n] < LIST_CAPS[n] {
                more.insert((*name).into(), json!(true));
            }
        }
        if !more.is_empty() {
            out.insert("more".into(), Value::Object(more));
        }
        out.insert("projects".into(), json!(projects));
        // The snooze presets to offer, on this PC's clock; listing them says this companion
        // also takes a typed time.
        let snooze: Vec<Value> = self
            .snooze_choices()
            .into_iter()
            .map(|c| {
                let mut entry = json!({ "k": c.key, "l": c.label });
                if let Some(when) = c.when {
                    entry["w"] = json!(when);
                }
                entry
            })
            .collect();
        out.insert("snz".into(), json!(snooze));
        // This companion takes a model, effort and permissions from the game, and what each
        // machine offers.
        out.insert("sel".into(), json!(true));
        let mut models: Vec<Value> = Vec::new();
        for (env, catalog) in self.catalogs.iter().filter(|(env, _)| env_index(env) > 0) {
            for provider in &catalog.providers {
                let mut entry = provider.clone();
                entry["e"] = json!(env_index(env));
                models.push(entry);
            }
        }
        if !models.is_empty() {
            out.insert("models".into(), json!(models));
        }
        let mut picks: Vec<Value> = Vec::new();
        for (env, catalog) in self.catalogs.iter().filter(|(env, _)| env_index(env) > 0) {
            for provider in &catalog.picks {
                let mut entry = provider.clone();
                entry["e"] = json!(env_index(env));
                picks.push(entry);
            }
        }
        if !picks.is_empty() {
            out.insert("picks".into(), json!(picks));
        }
        out.insert("view".into(), view.unwrap_or(Value::Null));
        out.insert("notes".into(), json!(self.notes));
        out.insert("events".into(), json!(self.events));
        if !self.outcomes.is_empty() {
            out.insert("out".into(), json!(self.outcomes.iter().map(|(_, o)| o).collect::<Vec<_>>()));
        }
        // A refused sign-in, else one that can only read: the game's top line says either.
        out.insert("auth".into(), json!(self.auth_error.as_deref().or(self.sign_in_read_only.then(|| crate::app::shown(READ_ONLY_LINE)))));
        // This companion pastes images (the game offers Ctrl+V, on a Mac Cmd+V, for them), and
        // the ones pasted so far, by the message they go with (left out when there are none).
        // Windows and macOS can read the clipboard (`clip::read_image`).
        #[cfg(any(windows, target_os = "macos"))]
        out.insert("pi".into(), json!(true));
        let pasted: Vec<Value> = self
            .drafts
            .iter()
            .flat_map(|(draft, images)| {
                images.iter().map(move |image| {
                    let mut entry = json!({ "d": draft, "i": image.n, "w": image.w, "h": image.h });
                    if let Some((slot, tw, th)) = image.thumb {
                        entry["f"] = json!(slot);
                        entry["tw"] = json!(tw);
                        entry["th"] = json!(th);
                    }
                    if let Some((slot, bw, bh)) = image.big {
                        entry["bf"] = json!(slot);
                        entry["bw"] = json!(bw);
                        entry["bh"] = json!(bh);
                    }
                    entry
                })
            })
            .collect();
        if !pasted.is_empty() {
            out.insert("att".into(), json!(pasted));
        }
        // What the tray setting blocks, so the window can say so (left out when nothing is).
        if let Some(code) = self.trust.code() {
            out.insert("trust".into(), json!(code));
        }
        // The game's frames aren't signed: it can only read until WoW restarts with the key.
        if self.unsigned {
            out.insert("uns".into(), json!(1));
        }
        // The addon version on disk, so the game can say when it needs a restart of WoW to
        // load newer files (left out when unknown).
        if let Some(version) = self.slots.addon_version() {
            out.insert("av".into(), json!(plain(&version, 40)));
        }
        // The T3 runs an older T3 Code than this release is made for: what to say about it
        // in full (t3u: the version it runs, and where the update is) and in a few words for
        // the window's status pill (t3s). Both left out otherwise.
        if let Some(older) = &self.older_t3 {
            out.insert("t3u".into(), json!(plain(&older.line(), 200)));
            out.insert("t3s".into(), json!(plain(&older.title(), 60)));
        }
        // This companion's version, so the game can say when the companion is older than
        // the addon (one updated from CurseForge, say) and needs an update itself.
        out.insert("cv".into(), json!(plain(crate::updater::current_version(), 40)));
        Value::Object(out)
    }

    pub fn publish(&mut self) {
        if self.link.wow_key.as_deref().unwrap_or("").is_empty() || self.link.ui_session.is_none() {
            return;
        }
        let t = self.now_ms();
        if t - self.last_publish_at < MIN_PUBLISH_GAP_MS {
            return;
        }
        let data = self.snapshot();
        // Each update spends one of the UI session's 200 slots. Urgent: what answers you
        // (acks, notes, a thread you just opened), alerts and errors; with the window closed
        // nothing else is worth a slot. Everything else waits until ROUTINE_GAP_MS after the
        // last update.
        let urgent = json!({
            "ack": data["ack"],
            "lid": data["lid"],
            "events": data["events"],
            "notes": data["notes"],
            "out": data["out"],
            "auth": data["auth"],
            "trust": data["trust"],
            "uns": data["uns"],
            "av": data["av"],
            "t3u": data["t3u"],
            "att": data["att"],
            "open": self.window_open,
            // A question appearing (or going away) in the open thread is urgent too.
            "view": if data["view"].is_null() {
                Value::Null
            } else {
                json!([data["view"]["id"], data["view"]["more"], data["view"]["ask"]["k"], data["view"]["approval"]])
            },
        });
        let hash = hash_of(&if self.window_open { without_times(&data) } else { urgent.clone() });
        if Some(hash) == self.last_published_hash {
            return;
        }
        let urgent_hash = hash_of(&urgent);
        if Some(urgent_hash) == self.last_urgent_hash && t - self.last_publish_at < ROUTINE_GAP_MS {
            return;
        }
        if self.link.slot > SLOT_COUNT {
            if self.slots_out_logged != self.link.ui_session {
                self.slots_out_logged = self.link.ui_session.clone();
                self.log("The game used all 200 update slots of this UI session: Reload UI in the game for more");
            }
            return;
        }
        let mut polling = self.link.polling == Some(true) || self.signals_out.is_some();
        let now_s = t.div_euclid(1000);
        // ns/nt: where and when the game's next wait starts. Polling doesn't move it.
        let mut wait = (self.link.signal, self.link.signal_time);
        let mut raise = Vec::new();
        if !polling {
            // The game may be a moment either side of moving to its next file, so raise the
            // file it should be watching now and both neighbours.
            let watched = self.watched_signal(now_s);
            if watched + 1 > SIGNAL_COUNT {
                // Past the last file: the game notices that too and loads slots on a timer.
                self.signals_out = Some(SIGNALS_OUT.into());
                self.log("This WoW session used all its signal files: updates come on a timer until WoW restarts");
                polling = true;
            } else {
                raise.extend(self.link.signal.max(watched.saturating_sub(1))..=watched + 1);
                wait = (watched + 2, now_s);
                if wait.0 + 1 > SIGNAL_COUNT {
                    // The next wait has no files left: this update (the last one a signal
                    // brings) tells the game to load slots on a timer from now on.
                    self.signals_out = Some(SIGNALS_OUT.into());
                    self.log("This WoW session used all its signal files: updates come on a timer until WoW restarts");
                }
            }
        }
        let mut inbox = data;
        inbox["now"] = json!(now_s);
        if let Some(reason) = &self.signals_out {
            inbox["sigoff"] = json!(reason);
        }
        // The same update without the parts the game holds, for the slots past the first
        // few. A part it carries anew isn't counted as held until the game says it took it.
        self.update_id = self.update_id.wrapping_add(1).max(1);
        let parts: BTreeMap<&'static str, String> = KEPT_PARTS.iter().map(|&part| (part, format!("{:012x}", hash_of(&inbox[part]) >> 16))).collect();
        self.held.retain(|part, hash| parts.get(part) == Some(hash));
        inbox["g"] = json!(self.update_id);
        inbox["hx"] = json!(parts);
        let mut slim = inbox.clone();
        let same: Vec<&str> = KEPT_PARTS.iter().copied().filter(|part| !inbox[*part].is_null() && self.held.contains_key(part)).collect();
        if let Some(fields) = slim.as_object_mut().filter(|_| !same.is_empty()) {
            for part in &same {
                fields.remove(*part);
            }
            fields.insert("same".into(), json!(same));
        }
        self.update_parts = parts;
        // Write a few slots ahead so a missed count on either side still finds fresh data,
        // and only then raise the signals that send the game to read them. Nothing counts
        // as published unless every write worked.
        let mut written = Ok(());
        for slot in self.link.slot..(self.link.slot + 3).min(SLOT_COUNT + 1) {
            let update = if slot <= WHOLE_SLOTS { &mut inbox } else { &mut slim };
            update["slot"] = json!(slot);
            update["ns"] = json!(wait.0);
            update["nt"] = json!(wait.1);
            written = written.and_then(|_| self.slots.write_slot(slot, &format!("T3WoW_Inbox = {}\n", to_lua(update))));
        }
        let slots_written = written.is_ok();
        for n in raise {
            written = written.and_then(|_| self.slots.raise_signal(n));
        }
        if let (true, Err(error)) = (slots_written, &written) {
            // The update is there but nothing can tell the game so: it waits on a signal
            // that never comes. Gone files (a missing folder) don't come back by retrying;
            // anything else gets a few tries first. Then the slots say to use the timer.
            self.signal_failures += 1;
            let lost = error.kind() == std::io::ErrorKind::NotFound;
            if lost || self.signal_failures >= WRITE_FAILURES_SHOWN {
                let reason = if lost {
                    signals_lost().to_string()
                } else {
                    format!("couldn't write the signal files ({error}); updates come on a timer until WoW restarts")
                };
                self.log(&format!("Couldn't raise the game's signal, so it gets updates on a timer: {reason}"));
                self.signals_out = Some(reason);
                self.signal_failures = 0;
                // Again, without signals this time: the slots now carry sigoff.
                return self.publish();
            }
        } else if written.is_ok() {
            self.signal_failures = 0;
        }
        self.last_publish_at = t;
        if let Err(error) = written {
            let message = format!("Couldn't write the game's update files: {error}");
            if self.write_error.as_ref() != Some(&message) {
                self.log(&message);
            }
            self.write_error = Some(message);
            self.write_failures += 1;
            return;
        }
        if self.write_error.take().is_some() {
            self.log("Writing the game's update files works again");
        }
        self.write_failures = 0;
        if !polling {
            self.link.slot += 1;
            self.link.signal = wait.0;
            self.link.signal_time = wait.1;
        }
        self.last_published_hash = Some(hash);
        self.last_urgent_hash = Some(urgent_hash);
        self.save_state();
    }

    /// Sends the current state again, as if the game had never had it (after Reinstall
    /// blanked the slot and signal files). The open thread is read again, as what was built
    /// for it, or is being built, may name image files that are blank now.
    pub fn republish(&mut self) {
        self.view_data = None;
        self.view_changed();
        self.last_published_hash = None;
        self.last_urgent_hash = None;
        self.held.clear();
        self.last_publish_at = 0;
    }

    /// Runs the game's queued commands. Each counts as done (acked) once it ran, and only
    /// for the UI session it came from: a newer session counts afresh.
    fn run_commands(&mut self) {
        while let Some((client_key, session, login, command, seen_at, signed)) = self.commands.pop_front() {
            let s = command["s"].as_u64().unwrap_or(0);
            self.command_seen_at = seen_at;
            let outcome = self.run(&client_key, login.as_deref(), command, signed);
            if Some(session.as_str()) == self.link.ui_session.as_deref() && Some(client_key.as_str()) == self.link.client_key.as_deref() {
                let entry = self.link.acked.entry(client_key).or_insert(0);
                *entry = (*entry).max(s);
                self.save_state();
                if let Some(outcome) = outcome {
                    let at = self.now_ms();
                    self.outcomes.push((at, outcome));
                }
            }
        }
    }

    /// A poll is under way: the caller should tick again soon.
    pub fn polling(&self) -> bool {
        self.poll_run.is_some()
    }

    /// One step: run the game's commands, read T3 when it's time, publish what changed.
    /// A poll runs a step (one request) at a time for up to TICK_BUDGET_MS, then gives the
    /// caller (and the game's next commands, the tray's controls) a turn.
    pub fn tick(&mut self) {
        self.run_commands();
        let now = self.now_ms();
        self.outcomes.retain(|(at, _)| now - at < OUTCOME_KEEP_MS);
        let excess = self.outcomes.len().saturating_sub(OUTCOMES_KEPT);
        self.outcomes.drain(..excess);
        let started = self.now_ms();
        loop {
            if self.poll_run.is_none() {
                if self.now_ms() < self.poll_at {
                    break;
                }
                self.start_poll();
            }
            self.poll_step();
            if self.poll_run.is_none() || self.now_ms() - started >= TICK_BUDGET_MS {
                break;
            }
        }
        // Halfway through a poll the state is half new: publish only once it has taken a
        // while, so an answer to a command still goes out.
        let now = self.now_ms();
        if self.poll_run.as_ref().is_none_or(|run| now - run.started >= PUBLISH_MIDPOLL_MS) {
            self.publish();
        }
    }
}

#[cfg(test)]
mod tests;
