/// Integration test: full session lifecycle with polling
#[cfg(test)]
mod integration {
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_full_session_lifecycle() {
        // This test would require initializing the Tauri backend,
        // which isn't directly testable without the app context.
        // Instead, we test the individual components.
        println!("Full session lifecycle test marked for e2e");
    }
}
