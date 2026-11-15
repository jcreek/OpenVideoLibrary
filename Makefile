.PHONY: build web rust browser
# Interim Makefile: the test and local-verification targets arrive with the
# operations tooling later in this stack.
web:
	npm ci --prefix web
	npm run check --prefix web
	npm run build --prefix web
rust:
	cargo fmt --all -- --check
	cargo clippy --locked --workspace --all-targets -- -D warnings
	cargo test --locked --workspace
build: web
	cargo build --locked --release --workspace
	mkdir -p bin
	cp target/release/ovl-server bin/ovl
browser: build
	npm run test:e2e --prefix web
