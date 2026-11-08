// Fixture CLI for the approved-operation tests. Compiled by rustc into a temp
// directory, never part of the plexi crate. It records every launch in a marker
// file in its working directory, so a test can prove a refused operation never
// started the program. Compile-time FIXTURE_VERSION distinguishes two builds.
use std::io::Write;

const VERSION: &str = env!("FIXTURE_VERSION");

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Ok(mut marker) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(".fixture-launched")
    {
        let _ = writeln!(marker, "{VERSION} {args:?}");
    }
    let mode = args.first().map(String::as_str).unwrap_or("");
    let rest = &args[args.len().min(1)..];
    match mode {
        "echo" => {
            let quoted: Vec<String> = rest.iter().map(|a| format!("{a:?}")).collect();
            println!(
                "{{\"version\":{VERSION:?},\"argv\":[{}]}}",
                quoted.join(",")
            );
        }
        "version" => println!("{{\"version\":{VERSION:?}}}"),
        "sleep" => {
            let ms: u64 = rest.first().and_then(|v| v.parse().ok()).unwrap_or(0);
            std::thread::sleep(std::time::Duration::from_millis(ms));
            println!("{{\"slept\":{ms}}}");
        }
        "spray" => {
            let n: usize = rest.first().and_then(|v| v.parse().ok()).unwrap_or(0);
            let chunk = vec![b'x'; 8192];
            let mut out = std::io::stdout().lock();
            let mut left = n;
            while left > 0 {
                let take = left.min(chunk.len());
                if out.write_all(&chunk[..take]).is_err() {
                    return;
                }
                left -= take;
            }
        }
        "spawn-child" => {
            let child = std::process::Command::new("/bin/sleep")
                .arg("60")
                .spawn()
                .expect("spawn child");
            let _ = std::fs::write(".child-pid", child.id().to_string());
            std::thread::sleep(std::time::Duration::from_secs(60));
        }
        "read" => match rest.first().map(std::fs::read_to_string) {
            Some(Ok(text)) => println!("{{\"content\":{text:?}}}"),
            Some(Err(error)) => {
                eprintln!("read failed: {error}");
                std::process::exit(3);
            }
            None => std::process::exit(2),
        },
        "exit" => std::process::exit(rest.first().and_then(|v| v.parse().ok()).unwrap_or(1)),
        "env" => {
            let mut keys: Vec<String> = std::env::vars().map(|(k, _)| k).collect();
            keys.sort();
            println!(
                "{{\"env\":{keys:?},\"cwd\":{:?}}}",
                std::env::current_dir().unwrap_or_default()
            );
        }
        "fake-dyld" => {
            eprintln!("dyld[1]: Library not loaded: @rpath/libfixture.dylib");
            std::process::exit(1);
        }
        "text" => println!("plain text output"),
        _ => {
            eprintln!("unknown mode {mode:?}");
            std::process::exit(64);
        }
    }
}
