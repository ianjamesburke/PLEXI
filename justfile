dev:
    cargo run

build:
    cargo build --release

run:
    cargo run --release

install:
    cargo build --release
    cp target/release/plexi /usr/local/bin/plexi
