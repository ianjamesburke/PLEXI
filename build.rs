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
    // Link libproc for proc_listchildpids used in shell::has_foreground_child.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-lib=proc");
    }
}
