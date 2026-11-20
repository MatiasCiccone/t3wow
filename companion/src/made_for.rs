//! The T3 Code this release is made for (package.json's `t3Code`, through build.rs), and what
//! to tell the user when the T3 the companion talks to is older.
//!
//! T3 Code can't be updated from outside: its MCP tools and its `t3code://` links have no way
//! to start one (`t3code://app/settings/about` reloads T3's window onto New thread), and its
//! desktop app updates itself (electron-updater) from **Check for Updates** in Settings >
//! About (or the same item in its menu: Help on Windows and Linux, the app menu on a Mac).
//! So the companion only says where that is: Settings > About, the same on every OS, with
//! the version T3 runs beside the button.

/// The T3 Code this release is made for, as T3 writes it ("0.1.9").
pub const T3_CODE: &str = env!("T3WOW_T3_CODE");

/// Where T3 Code's update is.
const WHERE: &str = "Settings > About > Check for Updates";

/// A T3 older than the one this release is made for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OlderT3 {
    /// The T3 Code it runs, as T3 reports it ("0.1.8").
    pub running: String,
}

impl OlderT3 {
    /// A few words: the tray notification's title and the game window's status pill.
    pub fn title(&self) -> String {
        format!("Update T3 Code to {T3_CODE}")
    }

    /// One line with the version it runs and where the update is: the tray menu's T3 line,
    /// the pill's tooltip and the game's chat.
    pub fn line(&self) -> String {
        format!("{} (it runs {}): in T3 Code, {WHERE}", self.title(), self.running)
    }

    /// The tray's notification and the terminal's longer note.
    pub fn note(&self) -> String {
        format!(
            "This T3 runs T3 Code {}. T3 WoW {} is made for T3 Code {T3_CODE}, and parts of it may not work with an older one. Update it in T3 Code from {WHERE}.",
            self.running,
            env!("CARGO_PKG_VERSION"),
        )
    }
}

/// When a T3 that reports this server version (`t3_environment_read`'s `serverVersion`) is
/// older than the one this release is made for: what it runs, as T3 writes it. A version that
/// doesn't read as numbers says nothing.
pub fn older_t3(server_version: &str) -> Option<OlderT3> {
    let running = numbers(server_version)?;
    let made_for = numbers(T3_CODE)?;
    (running < made_for).then(|| OlderT3 { running: server_version.trim().to_string() })
}

/// A version's numbers, without a `v` before or a pre-release after ("0.1.9-beta" is 0.1.9).
/// They compare part by part, so 0.1.10 is newer than 0.1.9.
fn numbers(version: &str) -> Option<[u64; 3]> {
    let core = version.trim().trim_start_matches('v').split(['-', '+']).next()?;
    let parts: Vec<u64> = core.split('.').map(|part| part.parse().ok()).collect::<Option<_>>()?;
    if parts.is_empty() || parts.len() > 3 {
        return None;
    }
    let mut out = [0; 3];
    out[..parts.len()].copy_from_slice(&parts);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_made_for_comes_from_package_json() {
        let package: serde_json::Value = serde_json::from_str(include_str!("../../package.json")).unwrap();
        assert_eq!(package["t3Code"].as_str(), Some(T3_CODE));
    }

    #[test]
    fn versions_compare_part_by_part() {
        assert!(numbers("0.1.10") > numbers("0.1.9"));
        assert!(numbers("0.2.0") > numbers("0.1.10"));
        assert_eq!(numbers("v0.1.9-beta.2"), Some([0, 1, 9]));
        assert_eq!(numbers("0.1"), Some([0, 1, 0]));
    }

    #[test]
    fn an_older_t3_gets_the_update_suggestion_and_a_current_or_newer_one_doesnt() {
        let made_for = numbers(T3_CODE).unwrap();
        assert!(made_for > [0, 1, 8], "made for T3 Code 0.1.9 or later");
        let older_version = "0.1.8";
        let found = older_t3(older_version).expect("an older T3");
        assert_eq!(found.running, "0.1.8");
        assert_eq!(found.title(), format!("Update T3 Code to {T3_CODE}"));
        assert_eq!(found.line(), format!("Update T3 Code to {T3_CODE} (it runs 0.1.8): in T3 Code, Settings > About > Check for Updates"));
        assert!(found.note().contains(&format!("made for T3 Code {T3_CODE}")), "{}", found.note());
        // A pre-release of an older one is older too; a pre-release of this one isn't.
        assert!(older_t3("v0.1.8-beta.2").is_some());
        let current = format!("{}.{}.{}", made_for[0], made_for[1], made_for[2]);
        assert_eq!(older_t3(&current), None);
        assert_eq!(older_t3(&format!("{current}-beta.1")), None);
        assert_eq!(older_t3(&format!("{}.{}.{}", made_for[0], made_for[1], made_for[2] + 1)), None);
        assert_eq!(older_t3("0.2.0"), None);
        // Part by part, as numbers: as text, 0.1.9 would read newer than 0.1.11 and 0.1.100 older.
        assert!(older_t3("0.1.9").is_some(), "0.1.9 is older than 0.1.10 and later");
        assert_eq!(older_t3("0.1.100"), None, "0.1.100 is newer than 0.1.11, not older");
    }

    #[test]
    fn a_version_that_doesnt_read_as_numbers_says_nothing() {
        assert_eq!(older_t3(""), None);
        assert_eq!(older_t3("dev"), None);
        assert_eq!(older_t3("0.1.x"), None);
    }
}
