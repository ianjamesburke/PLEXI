//! Compiled by the Windows launch regression as a no-argument GUI host stand-in.
#![windows_subsystem = "windows"]
fn main() {
    assert_eq!(
        std::env::args_os().count(),
        1,
        "executable path became arguments"
    );
    let report = std::env::var("HOST01_REPORT").unwrap();
    let pending = format!("{report}.pending");
    std::fs::write(&pending, std::process::id().to_string()).unwrap();
    std::fs::rename(pending, &report).unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !std::path::Path::new(&format!("{report}.stop")).exists()
        && std::time::Instant::now() < until
    {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}
