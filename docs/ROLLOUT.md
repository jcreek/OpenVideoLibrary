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
