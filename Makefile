.PHONY: build test web rust browser verify-local
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
test: web rust
	python3 -m unittest discover -s scripts -p 'test_*.py'
browser: build
	npm run test:e2e --prefix web

# Full hosted verify-job equivalent on this machine. FFmpeg/FFprobe, browser
# engines and an already-cached Nginx image are prerequisites; no image pull.
verify-local:
	bash scripts/verify-local.sh
