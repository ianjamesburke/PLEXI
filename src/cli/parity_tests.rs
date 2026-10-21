use clap::CommandFactory;
#[path = "../../tools/gen_cli_docs/src/parity.rs"]
mod generator;

#[test]
fn action_cli_parity_is_complete_and_fresh() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(root.join("src/host/keys.rs")).unwrap();
    let data = std::fs::read_to_string(root.join("docs/cli/parity.json")).unwrap();
    let generated = generator::generate(&super::args::Cli::command(), &source, &data).unwrap();
    assert_eq!(
        std::fs::read_to_string(root.join("docs/cli/PARITY.md")).unwrap(),
        generated,
        "regenerate with cargo run -p gen_cli_docs -- --parity"
    );
}

#[test]
fn action_cli_parity_rejects_unmapped_action() {
    let source = include_str!("../host/keys.rs").replace(
        "pub enum Action {",
        "pub enum Action { DummyUnmappedAction,",
    );
    let error = generator::generate(
        &super::args::Cli::command(),
        &source,
        include_str!("../../docs/cli/parity.json"),
    )
    .unwrap_err();
    assert!(error.contains("DummyUnmappedAction"), "{error}");
}
