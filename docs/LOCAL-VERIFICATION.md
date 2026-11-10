# Local and N100 verification

Verification runs on the development machine and over SSH on the N100. The
GitHub `Verify Rust` workflow is disabled, and its file has no push/PR triggers.
Do not re-enable or dispatch hosted workflows without explicit user approval to
spend Actions minutes. Existing run logs and artifacts remain historical evidence.

## Full local verification job

Install Rust from `rust-toolchain.toml`, Node/npm, Python 3, FFmpeg/FFprobe and the
Playwright browser engines. Install browser engines once when needed:

```sh
npm ci --prefix web
cd web && npx playwright install chromium firefox webkit && cd ..
```

With those tools on PATH and an already-cached Nginx image available to Docker:

```sh
OVL_NGINX_IMAGE=nginx:stable make verify-local
```

`OVL_NGINX_IMAGE` can instead be the exact cached image ID. The command performs
web install/check/build, Rust format/Clippy/tests, Python regressions, generated
API-type consistency, the separate benchmark crate check/format, explicit CPU
media tests, browser workflows and the disposable Nginx/TLS backup/restore drill.
It neither pulls Docker images nor launches hosted workflows. It stops on a
failure; later checks are unrun, not passed. Generated types are compared with
their pre-run content, so valid uncommitted contract updates are supported.

FFmpeg/FFprobe must be on PATH. For a nondefault Playwright cache set
`PLAYWRIGHT_BROWSERS_PATH`; set `PLAYWRIGHT_CHROMIUM_CHANNEL=chrome` only when
using an installed Chrome instead of Playwright Chromium. Browser engines and
OSes are distinct acceptance environments. A macOS pass does not erase a Linux
browser failure.

## Physical N100 checks

Use the isolated SSH runner with the existing trusted host/key:

```sh
python3 scripts/validate_n100.py --host USER@HOST --key KEY \
  --vaapi-driver-path /absolute/isolated/driver/dri --output RESULT.json
```

The optional driver override affects only the physical checks and does not
install or replace system drivers. It records source/driver identity and runs
Rust checks, CPU media, physical SDR and physical HDR separately. See `HDR.md`
for the full-driver requirement and the separate known-bad-driver rejection test.
This does not run performance acceptance or change shared services/mounts.

## Local Linux image acceptance

These are separate from `verify-local` and need sufficient Docker storage plus a
builder capable of the chosen architecture. Build/run each platform explicitly:

```sh
docker buildx build --platform linux/amd64 --load -t ovl:acceptance-amd64 .
python3 scripts/test_container.py --image ovl:acceptance-amd64 --output test-results/image-amd64.json
docker buildx build --platform linux/arm64 --load -t ovl:acceptance-arm64 .
python3 scripts/test_container.py --image ovl:acceptance-arm64 --output test-results/image-arm64.json
```

The local Docker VM is currently full, and the N100 has no usable isolated
container runtime. Fresh image acceptance therefore needs additional dedicated
Docker storage or another authorized local Linux runtime. Do not delete unrelated
images, volumes or data to make room. Earlier native Linux CI image results apply
only to their recorded source. No automatic hosted replacement will be started.

## Outstanding browser failure

The last hosted run at `9572feb` passed both image jobs but failed one Chromium
queue workflow (eight browser cases passed); its Nginx step was not reached.
The failure remains open. A deterministic delayed-player regression is in
progress locally; disabling hosted workflows does not remove or waive that check.
