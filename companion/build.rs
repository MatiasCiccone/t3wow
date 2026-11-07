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
        if let Err(error) = res.compile() {
            // A missing resource compiler only costs the icon; the exe still builds.
            println!("cargo:warning=no exe icon: {error}");
        }
    }
}
