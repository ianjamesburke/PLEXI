//! Compiled by the Windows launch regression as a no-argument GUI host stand-in.
#![windows_subsystem = "windows"]
use std::ffi::c_void;
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetHandleInformation(handle: *mut c_void, flags: *mut u32) -> i32;
}
fn main() {
    assert_eq!(
        std::env::args_os().count(),
        1,
        "executable path became arguments"
    );
    let handle: usize = std::env::var("HOST01_CAPTURE_HANDLE")
        .unwrap()
        .parse()
        .unwrap();
    let mut flags = 0;
    let inherited = unsafe { GetHandleInformation(handle as *mut c_void, &mut flags) } != 0;
    let report = std::env::var("HOST01_REPORT").unwrap();
    std::fs::write(&report, format!("{}:{}", std::process::id(), inherited)).unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !std::path::Path::new(&format!("{report}.stop")).exists()
        && std::time::Instant::now() < until
    {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}
