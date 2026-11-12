//! The T3 Code this release is made for (package.json's `t3Code`, through build.rs), and what
//! to tell the user when the T3 the companion talks to is older.
//!
//! T3 Code can't be updated from outside: its MCP tools and its `t3code://` links have no way
//! to start one (`t3code://app/settings/about` reloads T3's window onto New thread), and its
//! desktop app updates itself (electron-updater) from **Check for Updates** in Settings >
//! About (or the same item in its menu: Help on Windows and Linux, the app menu on a Mac).
//! So the companion only says where that is: Settings > About, the same on every OS, with
//! the version T3 runs beside the button.

/// The T3 Code this release is made for, as the docs write it ("0.19.0").
pub const T3_CODE: &str = env!("T3WOW_T3_CODE");

/// Where T3 Code's update is.
const WHERE: &str = "Settings > About > Check for Updates";

/// A T3 older than the one this release is made for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OlderT3 {
    /// The T3 Code it runs, as the docs write it ("0.18.0").
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
/// older than the one this release is made for: what it runs. A version that doesn't read as
/// numbers says nothing.
pub fn older_t3(server_version: &str) -> Option<OlderT3> {
    let running = numbers(server_version)?;
    let made_for = server_numbers(T3_CODE)?;
    (running < made_for).then(|| OlderT3 { running: t3_code_name(&running) })
}

/// A version's numbers, without a `v` before or a pre-release after ("0.1.9-beta" is 0.1.9).
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

/// The server version T3 Code reports for a release the docs name: the docs join the server
/// version's last two numbers, so T3 Code 0.19.0 reports 0.1.9 (0.20.0 would report 0.2.0,
/// 0.110.0 would report 0.1.10). A name that doesn't read that way is its own server version.
fn server_numbers(name: &str) -> Option<[u64; 3]> {
    let [major, minor, patch] = numbers(name)?;
    let digits = minor.to_string();
    if major != 0 || patch != 0 || digits.len() < 2 {
        return Some([major, minor, patch]);
    }
    let (first, rest) = digits.split_at(1);
    Some([0, first.parse().ok()?, rest.parse().ok()?])
}

/// The docs' name for a server version: 0.1.9 is T3 Code 0.19.0.
fn t3_code_name(server: &[u64; 3]) -> String {
    match server {
        [0, minor @ 1..=9, patch] => format!("0.{minor}{patch}.0"),
        [major, minor, patch] => format!("{major}.{minor}.{patch}"),
    }
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
    fn server_versions_read_as_the_t3_code_releases_the_docs_name() {
        assert_eq!(server_numbers("0.19.0"), Some([0, 1, 9]));
        assert_eq!(server_numbers("0.18.0"), Some([0, 1, 8]));
        assert_eq!(server_numbers("0.110.0"), Some([0, 1, 10]));
        assert_eq!(server_numbers("0.20.0"), Some([0, 2, 0]));
        assert_eq!(server_numbers("0.9.0"), Some([0, 9, 0]));
        assert_eq!(server_numbers("1.2.0"), Some([1, 2, 0]));
        assert_eq!(t3_code_name(&[0, 1, 8]), "0.18.0");
        assert_eq!(t3_code_name(&[0, 1, 10]), "0.110.0");
        assert_eq!(t3_code_name(&[0, 2, 0]), "0.20.0");
        assert_eq!(t3_code_name(&[1, 0, 0]), "1.0.0");
    }

    #[test]
    fn an_older_t3_gets_the_update_suggestion_and_a_current_or_newer_one_doesnt() {
        let made_for = server_numbers(T3_CODE).unwrap();
        assert!(made_for > [0, 1, 8], "made for T3 Code 0.19.0 or later");
        let older_version = "0.1.8";
        let found = older_t3(older_version).expect("an older T3");
        assert_eq!(found.running, "0.18.0");
        assert_eq!(found.title(), format!("Update T3 Code to {T3_CODE}"));
        assert_eq!(found.line(), format!("Update T3 Code to {T3_CODE} (it runs 0.18.0): in T3 Code, Settings > About > Check for Updates"));
        assert!(found.note().contains(&format!("made for T3 Code {T3_CODE}")), "{}", found.note());
        // A pre-release of an older one is older too; a pre-release of this one isn't.
        assert!(older_t3("v0.1.8-beta.2").is_some());
        let current = format!("{}.{}.{}", made_for[0], made_for[1], made_for[2]);
        assert_eq!(older_t3(&current), None);
        assert_eq!(older_t3(&format!("{current}-beta.1")), None);
        assert_eq!(older_t3(&format!("{}.{}.{}", made_for[0], made_for[1], made_for[2] + 1)), None);
        assert_eq!(older_t3("0.2.0"), None);
    }

    #[test]
    fn a_version_that_doesnt_read_as_numbers_says_nothing() {
        assert_eq!(older_t3(""), None);
        assert_eq!(older_t3("dev"), None);
        assert_eq!(older_t3("0.1.x"), None);
    }
}
