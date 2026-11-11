// Gives t3wow.exe its icon and version details (Explorer, taskbar, Task Manager), and its
// version string (T3WOW_VERSION).
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=assets/t3wow.ico");
    let version = build_version();
    println!("cargo:rustc-env=T3WOW_VERSION={version}");
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
/// build names its commit, "0.9.1-dev+57ee2ec", with ".dirty" when the companion or addon
/// has changes not committed yet, so a test build never passes for the release it started
/// from. Without git, "0.9.1-dev".
fn build_version() -> String {
    let base = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    println!("cargo:rerun-if-env-changed=T3WOW_RELEASE");
    println!("cargo:rerun-if-changed=Cargo.toml");
    if std::env::var_os("T3WOW_RELEASE").is_some_and(|v| !v.is_empty()) {
        return base;
    }
    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    let Some(commit) = git(&["rev-parse", "--short=7", "HEAD"]) else {
        return format!("{base}-dev");
    };
    // A new commit or checkout moves HEAD's reflog; an edit touches src or the addon.
    for path in ["HEAD", "logs/HEAD"] {
        if let Some(file) = git(&["rev-parse", "--git-path", path]) {
            println!("cargo:rerun-if-changed={file}");
        }
    }
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=../addon");
    let dirty = git(&["status", "--porcelain", "--", ".", "../addon"]).is_some_and(|changes| !changes.is_empty());
    format!("{base}-dev+{commit}{}", if dirty { ".dirty" } else { "" })
}
