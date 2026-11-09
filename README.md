# OpenVideoLibrary

An experimental Rust media server with a Svelte web client, built toward Jellyfin
capability parity with measured performance on an Intel N100 and SMB media.
**Rust preview, not release ready or full Jellyfin parity.** No Jellyfin speedup
or stream-capacity claim is established. See [status](docs/STATUS.md),
[parity matrix](docs/PARITY.md) and [architecture decision](docs/ADR-001-rust.md).

## Build and run

Requires the pinned Rust toolchain, Node 22+, Python 3, a C compiler for bundled
SQLite, and FFmpeg/FFprobe on PATH. FFmpeg needs libx264/AAC for software playback,
libass for subtitle burn-in, and zscale/tonemap for HDR conversion.

```sh
make build
./bin/ovl
```

Open http://localhost:8096, create the administrator account, add movie/TV folders,
and scan. Media mounts are read-only. Application state defaults to `./data`;
never run two servers on the same directory. The frontend is served from
`web/dist`; set `OVL_WEB` when starting the binary from another working directory.

The native `/api/v1` contract remains in [OpenAPI](docs/openapi.json). Existing
Jellyfin apps and plugin binaries are not supported; we build our own clients.
The web UI includes browsing, series, accounts/grants, progress, metadata,
audio/subtitles, playback settings, personal collections/playlists and queues.

## Configuration

| Variable | Default | Purpose |
| --- | --- | --- |
| `OVL_DATA` | `./data` | Local SQLite/cache/transcode directory |
| `OVL_LISTEN` | `:8096` | Listen address |
| `OVL_WEB` | `web/dist` | Built Svelte assets |
| `OVL_MAX_STREAMS` | `2` | Maximum FFmpeg sessions, clamped to 64 |
| `OVL_SECURE_COOKIES` | `false` | Set true behind HTTPS |
| `OVL_ACCELERATOR` | `auto` | `auto`, `qsv`, `vaapi`, `nvenc`, `cpu` |
| `OVL_TRANSCODE_BYTES` | `4294967296` | Per-session workspace reservation/budget |
| `OVL_MIN_FREE_BYTES` | `1073741824` | Free-space reserve |
| `OVL_SCAN_INTERVAL` | `6h` | Reconciliation interval, minimum one minute |
| `OVL_TMDB_KEY` | unset | Optional explicit TMDB metadata matching |
| `RUST_LOG` | application/info | Structured application/HTTP logging; use `ovl_server=debug,tower_http=info` for DB queue timing |

Hardware availability is established by bounded runtime encoder smoke tests,
not capacity certification. SDR hardware attempts can retry software before HLS
publication. HDR tone mapping and subtitle burn-in use bounded software pipelines;
N100 GPU and hardware HDR acceptance remain pending for Rust.

## Docker and Nginx

```sh
mkdir -p data
sudo chown 10001:10001 data
docker compose build
docker compose up -d
```

Adjust the media bind mount in `compose.yaml`; it must retain `:ro`. The supplied
Compose configuration requires HTTPS cookies and is intended for the existing
TLS reverse proxy. For isolated local HTTP development set
`OVL_SECURE_COOKIES=false`. See `docs/nginx.conf`. No image was published and the
new multi-architecture Docker build has not yet been certified.

## Development and validation

```sh
python3 scripts/generate-types.py
make test
# Actual generated-media software tests; distinct from GPU acceptance:
OVL_ACCELERATOR=cpu cargo test --locked --workspace -- --ignored
# Browser engines must be installed first:
cd web && npx playwright install chromium firefox webkit && cd ..
make browser
```

The repository has no Go source or Go build jobs. Historical benchmark reports
retain their original candidate labels and are not Rust production validation.
Browser engines do not certify every real mobile/TV device. Current evidence and
unrun checks are in [validation](docs/VALIDATION.md).

## Backups and migration

```sh
python3 scripts/backup.py backup --data ./data --output /backups/ovl-snapshot
python3 scripts/backup.py restore --source /backups/ovl-snapshot --data ./restored-data
```

SQLite v1 migrates transactionally to v2; existing v2 data, bcrypt passwords,
hashed sessions, grants and progress are preserved. Personal lists use additive
v2-compatible tables. Live WAL snapshots include those tables and cached artwork,
exclude transcodes and revoke sessions on restore. Restore requires a fresh
folder. Back up before switching; see [rollback](docs/ROLLOUT.md).

## Performance

Follow [the benchmark protocol](docs/BENCHMARKS.md). Initial targets remain
browse/search p95 <150 ms and LAN direct first decoded frame <2 s on the reference
N100/SMB deployment. Report workload, errors and repeated-run uncertainty alongside
CPU/RSS; separate server and FFmpeg resource use. Do not infer streaming capacity
or a Jellyfin speedup from warm API microbenchmarks.

## License

GNU Affero General Public License v3.0. See LICENSE. FFmpeg, dependencies and GPU
drivers retain their own licenses. Metadata integrations are optional and require
provider attribution and terms compliance.
