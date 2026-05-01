fn main() {
    let pkg_name = std::env::var("CARGO_PKG_NAME").unwrap();
    let title: String = pkg_name
        .split('-')
        .map(|word| {
            let mut chars = word.chars();
            chars
                .next()
                .map(|f| f.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ");
    println!("cargo:rustc-env=PLEXI_APP_TITLE={title}");
}
