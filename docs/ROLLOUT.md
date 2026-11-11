# Rust preview switch and rollback

1. Build and run checks using disposable data first. Run generated-media tests,
   the browser workflow and the target N100/device matrix before production use.
2. Create a format-1 backup with `scripts/backup.py`; verify restore into a new
   folder. Keep that snapshot outside application data and media mounts.
3. Stop the previous server and confirm its process has exited. The shared
   `server.lock` protocol prevents simultaneous access; never bypass it.
4. For a trial, restore the snapshot into a fresh directory, retain the original
   media paths and start Rust with `OVL_DATA` pointing at the restored data. Restored
   sessions are revoked; sign in again. Set the built frontend path and cookie mode.
5. Check account permissions, library status, sampled watch history and lists;
   verify direct/remux/transcode modes and read-only mounts. Validate reverse proxy
   forwarding, request limits, timeouts and TLS on the real deployment.
6. Cut over only after acceptance. The Rust-only repository does not ship the
   previous application; retain its known binary externally if rollback is needed.
7. To roll back, stop Rust, restore the pre-switch snapshot to another fresh
   folder and start the retained previous server against that folder. Changes
   after the snapshot will not appear. Additive list tables remain compatible with
   v2 backups, but older apps do not provide list UI or queue behavior.

Rust preview is not the stable release. A stable release requires broader media
and subtitle/HDR quality validation, real mobile browsers, physical GPU tests,
sustained/cold-cache N100 performance, multi-architecture images, Nginx deployment
validation and external security review. Registry publication is a separate action.

A disposable native proxy drill can run over SSH without a container runtime
or system package installation. In a test workspace on N100, download and
extract a version available in the host's trusted APT index. The recorded
version was `1.24.0-2ubuntu7.19`; verify the downloaded package hash against
the index and retain its identity.

```sh
apt-get download nginx=1.24.0-2ubuntu7.19
dpkg-deb -x nginx_1.24.0-2ubuntu7.19_amd64.deb ./nginx-test
# From the staged source checkout, as an unprivileged user:
python3 scripts/test_nginx.py --binary target/release/ovl-server \
  --nginx-binary /absolute/path/to/nginx-test/usr/sbin/nginx \
  --output nginx-native-acceptance.json
```

The drill renders the repository's Nginx locations into temporary TLS
configuration, binds both processes to loopback and tests snapshot rollback
after a trial queue edit. It uses the same Rust binary before and after the
rollback. Production TLS configuration, a rollback to the retained older
binary and the read-only production media mount still need an isolated
deployment drill.

The Docker Desktop VM had zero free space at the last check. No images,
volumes or build caches were deleted. For current-source image acceptance,
provide more Docker storage or an isolated Docker builder and runtime with
adequate space. Build and test each platform separately, using disposable
tags and keeping the media fixture mounted read-only:

```sh
docker buildx build --platform linux/amd64 --load -t ovl-acceptance:amd64 .
python3 scripts/test_container.py --image ovl-acceptance:amd64 --output linux-amd64.json
docker buildx build --platform linux/arm64 --load -t ovl-acceptance:arm64 .
python3 scripts/test_container.py --image ovl-acceptance:arm64 --output linux-arm64.json
```

Use native target runtimes for platform acceptance. The tested N100 account
has no container runtime and cannot create a user namespace. Its household
SMB mount is writable; leave it untouched and provision a separate read-only
test mount before SMB or production-host acceptance.

### Registered library paths

A library is registered with its canonical path. If that path later resolves
elsewhere, media access fails closed and scanning marks the library offline.
Restore the directory or read-only mount at the recorded path. Queue entries
and library rows are preserved.

An older database can contain a deliberately symlinked or otherwise
noncanonical root. The preview keeps its records but does not automatically
trust the current symlink target. Take a verified backup, then identify the
library ID and independently confirm the intended canonical media root.
Repair into a new data directory under a private, user-owned parent:

```sh
python3 scripts/repair_library_path.py --backup /backups/pre-switch \
  --output /private/staging/repaired --library-id 4 --root /media/canonical
```

The command takes a format-1 backup of a v2 database. It restores privately,
verifies each relative file path under the target root, then updates only
the library and file paths in one transaction. The IDs, grants, progress,
lists and credentials survive; restored sessions are revoked. It does not
touch the source data, backup or media. It refuses an existing destination
and stops on missing or redirected files, size changes or unique-path
conflicts. Size matching does not verify content identity. The operator must
verify the mapping and keep the media tree stable while repairing it.

Only start the server after the command succeeds; publishing the new
directory is not atomic. The path checks admit up to one million records
and have a cooperative 30-minute budget after restore; a kernel-blocked
filesystem call can still depend on mount timeouts. Start the repaired copy,
scan and verify the original IDs, grants, progress and lists before cutover.
Deleting and recreating the library is not an equivalent repair.
