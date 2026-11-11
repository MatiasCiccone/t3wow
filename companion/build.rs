// Gives t3wow.exe its icon and version details (Explorer, taskbar, Task Manager), and its
// version string (T3WOW_VERSION) and the T3 Code it is made for (T3WOW_T3_CODE).
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=assets");
    let version = build_version();
    println!("cargo:rustc-env=T3WOW_VERSION={version}");
    made_for_t3();
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/t3wow.ico");
        res.set("FileDescription", "T3 WoW companion");
        res.set("ProductName", "T3 WoW");
        res.set("ProductVersion", &version);
        res.set("CompanyName", "T3 WoW");
        res.set("LegalCopyright", "MIT License");
        // Common Controls 6, so the tray's address box and message boxes have the current
        // Windows look (and the edit box its placeholder text). Nothing else: DPI awareness
        // stays with the capture thread (capture.rs).
        res.set_manifest(
            r#"<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
<dependency><dependentAssembly>
<assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
</dependentAssembly></dependency>
</assembly>"#,
        );
        if let Err(error) = res.compile() {
            // A missing resource compiler only costs the icon; the exe still builds.
            println!("cargo:warning=no exe icon: {error}");
        }
    }
}

/// The release workflow (T3WOW_RELEASE set) builds the plain version, "0.9.1". Any other
/// build names its commit, "0.9.1-dev+57ee2ec", and when the companion or addon has changes
/// not committed yet, ".dirty" and a fingerprint of them ("0.9.1-dev+57ee2ec.dirty.3fa9c01"),
/// so a test build never passes for the release it started from, nor for another test build.
/// Without git, "0.9.1-dev".
fn build_version() -> String {
    let base = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    println!("cargo:rerun-if-env-changed=T3WOW_RELEASE");
    println!("cargo:rerun-if-changed=Cargo.toml");
    if std::env::var_os("T3WOW_RELEASE").is_some_and(|v| !v.is_empty()) {
        return base;
    }
    let git_bytes = |args: &[&str]| Command::new("git").args(args).output().ok().filter(|out| out.status.success()).map(|out| out.stdout);
    let git = |args: &[&str]| git_bytes(args).map(|out| String::from_utf8_lossy(&out).trim().to_string());
    let Some(commit) = git(&["rev-parse", "--short=7", "HEAD"]) else {
        return format!("{base}-dev");
    };
    // A new commit or checkout moves HEAD's reflog; an edit touches src, assets or the addon.
    for path in ["HEAD", "logs/HEAD"] {
        if let Some(file) = git(&["rev-parse", "--git-path", path]) {
            println!("cargo:rerun-if-changed={file}");
        }
    }
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=../addon");
    let paths = ["--", ".", "../addon"];
    if git(&[&["status", "--porcelain"][..], &paths].concat()).is_none_or(|changes| changes.is_empty()) {
        return format!("{base}-dev+{commit}");
    }
    // The uncommitted changes: tracked files' diff, and new files with their contents.
    let mut changes = git_bytes(&[&["diff", "HEAD", "--binary"][..], &paths].concat()).unwrap_or_default();
    for file in git(&[&["ls-files", "--others", "--exclude-standard"][..], &paths].concat()).unwrap_or_default().lines() {
        changes.extend_from_slice(file.as_bytes());
        changes.extend(std::fs::read(file).unwrap_or_default());
    }
    // FNV-1a, 28 bits: seven hex digits, like the commit.
    let hash = changes.iter().fold(0x811c_9dc5u32, |h, &b| (h ^ b as u32).wrapping_mul(0x0100_0193));
    format!("{base}-dev+{commit}.dirty.{:07x}", hash >> 4)
}

/// The T3 Code this release is made for: package.json's `t3Code`, the one place it's written
/// (the docs that name it are checked against it). The companion reads it as `T3WOW_T3_CODE`.
fn made_for_t3() {
    println!("cargo:rerun-if-changed=../package.json");
    let package = std::fs::read_to_string("../package.json").expect("package.json, next to the companion folder");
    let field = package.split("\"t3Code\"").nth(1).expect("package.json has no t3Code (the T3 Code this release is made for)");
    let version = field.split('"').nth(1).filter(|v| !v.is_empty() && v.split('.').all(|n| n.parse::<u64>().is_ok())).expect("package.json's t3Code reads like 0.19.0");
    println!("cargo:rustc-env=T3WOW_T3_CODE={version}");
}
