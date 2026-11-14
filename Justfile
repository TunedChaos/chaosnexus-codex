# chaosnexus-codex/Justfile

fetch:
    cargo fetch

build:
    cargo build

build-embedded:
    cargo build --features embed-docs

test:
    cargo test

clean:
    cargo clean
