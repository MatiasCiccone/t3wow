# Security

## Reporting a vulnerability

Report it privately, on GitHub: open this repository's **Security** tab and choose **Report a vulnerability** ([private vulnerability reporting](https://github.com/MatiasCiccone/t3wow/security/advisories/new)). Only you and the maintainers see the report. Please don't open a public issue or pull request for it.

Include what you can of:

- what's affected: the companion (and its version, from the tray menu or `t3wow.exe status`), the addon, or a release;
- Windows or macOS, and which version;
- how to reproduce it, and what someone could do with it.

The fix ships in a new release, which companions update to as usual. The advisory is published once that release is out.

## In scope

- **The companion**: `t3wow.exe` on Windows and `T3 WoW.app` on a Mac (`companion/`). That includes the commands it takes from the game and the **Commands from the game** setting, the T3 sign-in and the saved sign-ins, the clipboard read for pasting images and the key check that gates it, the pages it draws in a headless browser, and the files it writes in WoW's folder.
- **The addon** (`addon/T3WoW`).
- **Release and update signing**: the updater's checks (the SHA-256 file, the release key's signature, the Mac app's code signature), the Release workflow (`.github/workflows/release.yml`), and how the release key and certificates are kept.

The README's [Trust](README.md#trust) section describes limits that are known and documented, such as any addon being able to draw the command strip, and the strip being readable on a stream or recording. A way past what it says holds (the **Commands from the game** setting, the paste check, the access your T3 sign-in grants) is in scope.

Out of scope: T3 Code itself (report those to T3 Code), and World of Warcraft.

## Supported versions

Only the latest release. Fixes aren't backported: update to the latest release (the tray menu's **Check for updates**, or the menu bar's on a Mac).

## More

- [Trust](README.md#trust), in the README: what an addon can and can't do through T3 WoW, and the settings that limit it.
- [docs/release-signing.md](docs/release-signing.md): how releases and updates are signed and checked.
