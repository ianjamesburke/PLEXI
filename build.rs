fn main() {
    let channel = std::fs::read_to_string(".channel")
        .unwrap_or_default()
        .trim()
        .to_string();
    let title = match channel.as_str() {
        "alpha" => "Plexi Alpha",
        "beta" => "Plexi Beta",
        _ => "Plexi",
    };
    println!("cargo:rustc-env=PLEXI_APP_TITLE={title}");
    println!("cargo:rerun-if-changed=.channel");

    // Embed assets/app-icon.ico into plexi.exe so the icon shows in Explorer,
    // the taskbar, Task Manager, Start Menu shortcuts, and Add/Remove Programs.
    // No-op on non-Windows hosts (winresource isn't in scope there).
    #[cfg(target_os = "windows")]
    {
        println!("cargo:rerun-if-changed=assets/app-icon.ico");
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/app-icon.ico");
        res.compile()
            .expect("failed to embed Windows icon resource");
    }
}
