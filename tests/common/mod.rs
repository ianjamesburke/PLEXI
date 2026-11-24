//! The one way an integration test names the Plexi binary it spawns.
//!
//! Release builds resolve the profile from the account database and ignore
//! `HOME`, so a spawned release binary would reach the real profile instead of
//! the test's temporary home. `cargo test` compiles the binary and this test
//! crate in the same profile, so debug assertions here mean the binary honors
//! `HOME` (see `config::profile_home::honors_home`).

pub fn plexi_binary() -> &'static str {
    assert!(
        cfg!(debug_assertions),
        "refusing to spawn plexi: a release-profile binary ignores HOME and would reach the \
         real ~/.plexi* profile. Run these tests in the dev profile (plain `cargo test`)."
    );
    env!("CARGO_BIN_EXE_plexi")
}
