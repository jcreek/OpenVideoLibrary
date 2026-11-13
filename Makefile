.PHONY: rust
# Interim Makefile: the web, browser, image and local-verification targets
# arrive with the web client and operations tooling later in this stack.
rust:
	cargo fmt --all -- --check
	cargo clippy --locked --workspace --all-targets -- -D warnings
	cargo test --locked --workspace
