// Gives t3wow.exe its icon and version details (Explorer, taskbar, Task Manager).
fn main() {
    println!("cargo:rerun-if-changed=assets/t3wow.ico");
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/t3wow.ico");
        res.set("FileDescription", "T3 WoW companion");
        res.set("ProductName", "T3 WoW");
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
