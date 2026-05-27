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
        let icon = "assets/app-icon.ico";
        println!("cargo:rerun-if-changed={icon}");
        // Guard on existence (symmetric with the installer's HasIcon gate) so a
        // missing asset doesn't abort the build; a present-but-broken icon still fails loudly.
        if std::path::Path::new(icon).exists() {
            let mut res = winresource::WindowsResource::new();
            res.set_icon(icon);
            res.compile().expect("failed to embed Windows icon resource");
        } else {
            println!("cargo:warning={icon} not found — building without an embedded icon");
        }
    }
}
