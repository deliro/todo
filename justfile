default: check

fmt:
    cargo fmt

clippy:
    cargo clippy --all-targets --all-features -- -D warnings

check: fmt clippy

test:
    RUST_BACKTRACE=1 cargo test
