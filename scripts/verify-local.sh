#!/usr/bin/env bash
# Runs verification on the current host, without GitHub Actions or image pulls.
set -euo pipefail
cd "$(dirname "$0")/.."
make test
python3 - <<'PY'
from pathlib import Path
import subprocess
import sys
path = Path('web/src/api.generated.ts')
before = path.read_bytes()
subprocess.run([sys.executable, 'scripts/generate-types.py'], check=True)
if path.read_bytes() != before:
    raise SystemExit('Generated API types were stale; regenerated file is ready for review. Rerun verification after reviewing it.')
PY
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo check --locked --manifest-path bench/rust/Cargo.toml
cargo fmt --manifest-path bench/rust/Cargo.toml -- --check
OVL_ACCELERATOR=cpu cargo test --locked --workspace -- --ignored
cargo build --locked --package ovl-server
mkdir -p bin
cp target/debug/ovl-server bin/ovl
npm run test:e2e --prefix web
python3 scripts/test_nginx.py --image "${OVL_NGINX_IMAGE:-nginx:stable}" --output test-results/nginx-acceptance.json
