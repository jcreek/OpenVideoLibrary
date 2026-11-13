#!/usr/bin/env bash
# Run on the reference Ubuntu host. This never changes existing media servers or media.
set -euo pipefail
cd "$(dirname "$0")/.."
if [ "$#" -ne 3 ]; then
  echo 'Usage: bash scripts/reference-bench.sh /path/to/output /path/to/sample-video /path/to/read-only-media-root' >&2
  exit 2
fi
ovl_output="$1"
ovl_sample="$2"
ovl_media_root="$3"
for ovl_command in cargo ffmpeg python3; do command -v "$ovl_command" >/dev/null; done
mkdir -p "$ovl_output" bin
{
  date -u
  uname -a
  lscpu
  free -h
  cargo --version
  rustc --version
  ffmpeg -version
  findmnt -T "$ovl_media_root"
  git rev-parse HEAD 2>/dev/null || true
} > "$ovl_output/environment.txt"
cargo build --locked --release --manifest-path bench/rust/Cargo.toml
python3 scripts/bench_suite.py --requests 10000 --repetitions 5 --concurrency 1 10 25 50 100 --output "$ovl_output/idle.json"
python3 scripts/bench_suite.py --requests 10000 --repetitions 5 --concurrency 1 10 25 50 100 --media "$ovl_sample" --scan-root "$ovl_media_root" --ffmpeg-count 1 --output "$ovl_output/mixed.json"
printf '%s\n' 'Prototype measurements complete. This does not certify playback capacity or establish performance superiority.'
