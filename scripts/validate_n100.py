#!/usr/bin/env python3
"""Stage and validate Rust in a new isolated SSH workspace; no sudo or media writes.

Example: python3 scripts/validate_n100.py --host USER@HOST --key KEY --workspace /ABSOLUTE/WORKSPACE --output RESULT.json
Uses strict existing SSH host trust. Does not benchmark, evict caches, change
services, install OS packages or touch the historical benchmark workspace.
"""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import shlex
import subprocess
import time

MARKER = '.ovl-validate-staging'


def source_hash():
    digest = hashlib.sha256()
    paths = [Path('Cargo.toml'), Path('Cargo.lock'), Path('rust-toolchain.toml')]
    # Covers the Rust sources and the scripts the remote tests run (not web/ or docs/),
    # skipping the noise files rsync does not stage.
    for root in ('crates', 'scripts'):
        paths += sorted(p for p in Path(root).rglob('*')
                        if p.is_file() and p.name != '.DS_Store' and not p.name.startswith('._')
                        and '__pycache__' not in p.parts)
    for path in paths:
        digest.update(str(path).encode())
        digest.update(path.read_bytes())
    return digest.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--host', required=True)
    parser.add_argument('--key', required=True)
    parser.add_argument('--workspace', required=True, help='absolute path of the isolated remote workspace')
    parser.add_argument('--accelerator', choices=['vaapi', 'qsv', 'nvenc'], default='vaapi')
    parser.add_argument('--vaapi-driver-path', help='Process-only isolated VAAPI driver override; does not install or replace system drivers')
    parser.add_argument('--output', required=True)
    args = parser.parse_args()
    if args.host.startswith('-') or not args.workspace.startswith('/') or args.workspace == '/':
        parser.error('host and absolute isolated workspace required')
    if args.vaapi_driver_path and (args.accelerator != 'vaapi' or not args.vaapi_driver_path.startswith('/') or args.vaapi_driver_path == '/'):
        parser.error('driver override requires VAAPI and an absolute isolated directory')
    # Resolve caller-relative paths, then stage and hash from the repository root.
    key = str(Path(args.key).expanduser().resolve())
    output = Path(args.output).resolve()
    os.chdir(Path(__file__).resolve().parent.parent)
    source_sha256 = source_hash()
    ssh = ['ssh', '-o', 'BatchMode=yes', '-o', 'StrictHostKeyChecking=yes',
           '-o', 'IdentitiesOnly=yes', '-o', 'ConnectTimeout=5', '-i', key]
    destination = args.workspace.rstrip('/') + '/source'
    # rsync --delete prunes the destination, so only stage into a new directory or one
    # this script marked earlier; refuse symlinks and unrelated application checkouts.
    marker = destination + '/' + MARKER
    preflight = ('test ! -L ' + shlex.quote(destination) + ' && { test ! -e ' + shlex.quote(destination)
                 + ' || test -f ' + shlex.quote(marker) + '; } && test ! -e ' + shlex.quote(destination + '/go.mod')
                 + ' && mkdir -p ' + shlex.quote(destination) + ' && touch ' + shlex.quote(marker))
    subprocess.run(ssh + [args.host, preflight], check=True, timeout=15)
    # Root-only excludes are anchored so same-named crate paths are staged; web build
    # outputs stay excluded at any depth. --delete keeps the remote tree equal to the
    # local one; excluded paths such as the remote target cache are kept.
    excludes = ['/' + MARKER, '/.git', '/target', '/bench/rust/target', '/bin', '/data',
                'node_modules', 'test-results', 'playwright-report', '.DS_Store', '._*',
                '__pycache__', '.env', '.env.*', '*.db', '*.db-wal', '*.db-shm']
    subprocess.run(['rsync', '-az', '--delete', *['--exclude=' + v for v in excludes],
                    '-e', shlex.join(ssh), './', args.host + ':' + shlex.quote(destination) + '/'],
                   check=True, timeout=120)
    report = {'recorded_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
              'source_sha256': source_sha256,
              'source_hash_scope': 'Cargo files, crates/ and scripts/', 'scope': 'SSH N100 Linux tests; no benchmark certification',
              'driver_override': args.vaapi_driver_path, 'checks': [], 'complete': False}
    output.parent.mkdir(parents=True, exist_ok=True)
    checks = [
        ('environment', 'uname -a; lscpu; free -b; rustc --version; cargo --version; ffmpeg -version; readlink -f /sys/class/drm/renderD128/device/driver; dpkg-query -W intel-media-va-driver mesa-va-drivers; if command -v vainfo >/dev/null; then vainfo --display drm --device /dev/dri/renderD128; else printf "vainfo unavailable; physical graph tests recorded separately\\n"; fi'),
        ('format', 'cargo fmt --all -- --check'),
        ('clippy', 'cargo clippy --locked --workspace --all-targets --all-features -- -D warnings'),
        ('integration', 'cargo test --locked --workspace'),
        ('software-media', 'OVL_ACCELERATOR=cpu cargo test --locked --workspace -- --ignored'),
        ('physical-sdr', 'OVL_ACCELERATOR=' + args.accelerator + ' cargo test --locked --features gpu-tests --test hardware physical_sdr -- --ignored --nocapture'),
    ]
    if args.accelerator == 'vaapi':
        checks.append(('physical-hdr', 'OVL_ACCELERATOR=vaapi OVL_HDR_ACCELERATOR=vaapi cargo test --locked --features gpu-tests --test hardware physical_vaapi_hdr -- --ignored --nocapture'))
    if args.vaapi_driver_path:
        driver = args.vaapi_driver_path.rstrip('/') + '/iHD_drv_video.so'
        checks.insert(1, ('driver-override', 'sha256sum ' + shlex.quote(driver)))
        prefix = 'LIBVA_DRIVER_NAME=iHD LIBVA_DRIVERS_PATH=' + shlex.quote(args.vaapi_driver_path) + ' '
        checks = [(name, prefix + command if name.startswith('physical-') else command) for name, command in checks]
    failed = False
    for name, command in checks:
        start = time.monotonic()
        remote = 'timeout --signal=TERM --kill-after=15s 840s sh -c ' + shlex.quote('cd ' + shlex.quote(destination) + ' && export PATH="$HOME/.cargo/bin:$PATH" && ' + command)
        try:
            result = subprocess.run(ssh + [args.host, remote], capture_output=True, text=True, timeout=900)
            row = {'name': name, 'command': command, 'exit_code': result.returncode,
                   'seconds': time.monotonic() - start, 'stdout': result.stdout, 'stderr': result.stderr}
        except subprocess.TimeoutExpired:
            # SSH disconnection does not establish that remote work terminated.
            row = {'name': name, 'command': command, 'seconds': time.monotonic() - start,
                   'exit_code': None, 'error': 'Observation timed out; inspect remote processes before retrying.'}
        report['checks'].append(row)
        output.write_text(json.dumps(report, indent=2) + '\n')
        print(name, row['exit_code'], flush=True)
        if row['exit_code'] != 0:
            failed = True
            if row['exit_code'] is None:
                break
    report['complete'] = not failed
    output.write_text(json.dumps(report, indent=2) + '\n')
    if failed:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
