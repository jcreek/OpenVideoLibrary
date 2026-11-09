# HDR playback and Intel VAAPI acceptance

Software tone mapping remains the default. `OVL_HDR_ACCELERATOR=vaapi` opts in to
hardware HDR10 conversion when `OVL_ACCELERATOR` is `auto` or `vaapi`. A forced
`OVL_ACCELERATOR=cpu` always takes precedence. Restart after changing driver or
accelerator configuration; runtime capability results are cached per process.

The admitted hardware slice is HEVC, 10-bit 4:2:0, limited range, PQ transfer,
BT.2020 primaries/nonconstant matrix, with mastering-display and content-light
metadata. A bounded eight-packet frame probe supplements stream metadata;
unsuccessful frame inspection cannot admit the hardware path. Explicit Dolby
Vision tags/side data, unknown/full range, HLG and subtitle burn-in are excluded
from this slice. HLG and burn-in retain software conversion; Dolby Vision support
and quality remain unverified, rather than being established by that fallback.
Ordinary CPU playback does not run the extra frame-metadata probe.

The graph uses VAAPI decoding, `tonemap_vaapi`, scaling and H.264 encoding, with
AAC audio. Existing stream admission, storage reservation, process deadlines,
cancellation and reaping apply. Hardware failure can retry CPU only before master
publication. Published segments are never replaced by a new CPU attempt.

## Silent-black driver safeguard

Successful FFmpeg exit is insufficient evidence. Before the first eligible
hardware session, a runtime picture probe converts a tiny calibrated PQ fixture
through the production graph and software-decodes its output. It checks neutral
black, shadow, midtone and highlight separation. A driver that returns black
video is rejected and the session uses CPU. This is a fault guard, not HDR
perceptual certification. The probe runs in the admitted session's owned workspace,
requires a 1 MiB budget/free-space allowance and has three sequential subprocesses,
each with an eight-second deadline. Session cancellation terminates/reaps active
probe work and session teardown removes its workspace. Optional detection busy
or unavailable results use CPU; shutdown cancellation remains cancellation.

## N100 driver finding

On the reference N100, Ubuntu's installed `intel-media-va-driver`
`24.1.0+dfsg1-1ubuntu0.2` completed seven VAAPI graph variants successfully but
returned black decoded pictures. Reordering scaling, using software decode/upload,
changing surface format and explicitly setting color properties did not fix it.
The runtime guard rejects this driver. The earlier exit/decode-only feasibility
probe did not catch this quality failure and is not HDR acceptance.

Ubuntu's `intel-media-va-driver-non-free` `24.1.0+ds1-1`, unpacked into an isolated
user-owned test directory, produced distinct calibrated luminance patches in the
same seven graphs. It was selected only through process-local
`LIBVA_DRIVER_NAME=iHD` / `LIBVA_DRIVERS_PATH`. No OS package was installed, no
system driver was replaced, and no household service or mount was changed.
The archive occupies about 43 MiB including extracted files. Driver hashes and
commands are recorded in the graph and final validation artifacts. Intel documents
[free/full shader builds](https://github.com/intel/media-driver/wiki/Media-Driver-Shaders-%28EU-Kernels%29);
Ubuntu publishes the [full driver package](https://packages.ubuntu.com/noble/intel-media-va-driver-non-free).

The physical corpus uses actual ST 2084 luminance values, rather than simply
retagging an SDR pattern. It checks nine grayscale levels from 0 to 1,000 nits,
three primary-color tiles, first/middle/final decoded frames, both output sizes,
seeking past an initial black interval, alternate 880 Hz audio, BT.709 tags and
the authenticated playback route. The session must record VAAPI, so CPU fallback
cannot pass the hardware assertion. These synthetic checks do not establish
creative intent, real-film perceptual quality, all HDR metadata variations, HLG
hardware, Dolby Vision, other GPUs, real displays or sustained performance.

## Reproduce without replacing system drivers

After unpacking the trusted Ubuntu package into a new isolated test directory,
use its `usr/lib/x86_64-linux-gnu/dri` directory only for the test processes:

```sh
LIBVA_DRIVER_NAME=iHD LIBVA_DRIVERS_PATH=/absolute/isolated/driver/dri \
  OVL_ACCELERATOR=vaapi OVL_HDR_ACCELERATOR=vaapi \
  cargo test --locked --features gpu-tests --test hardware \
  physical_vaapi_hdr -- --ignored --nocapture
```

`python3 scripts/validate_n100.py --host USER@HOST --key KEY \
--vaapi-driver-path /absolute/isolated/driver/dri --output RESULT.json` stages the
Rust source in the isolated SSH workspace and records driver identity, software,
SDR and HDR checks separately. The override affects only physical checks.

For the explicitly known-broken system driver, run the negative test separately:

```sh
env -u LIBVA_DRIVER_NAME -u LIBVA_DRIVERS_PATH OVL_EXPECT_HDR_REJECTION=true \
  cargo test --locked --features gpu-tests --test hdr_driver_rejection \
  -- --ignored --nocapture
```

CPU regression coverage is separate and needs no GPU:

```sh
OVL_ACCELERATOR=cpu cargo test --locked --workspace -- --ignored
```

That suite includes simulated driver faults through a real HTTP server: CPU
precedence, black-picture rejection, pre-publication retry, preservation after
publication, DELETE during a hanging probe and SIGTERM during a hanging probe.
The DELETE case verifies child disappearance/workspace removal while the server
is still running; shutdown cannot mask ineffective session cancellation.

The default container's free Intel driver is not an HDR qualification. A deployed
full driver must match the container OS/ABI and pass physical acceptance there;
copying the tested Ubuntu binary into the Debian image is not a verified solution.
No full-driver binary or registry image is published by this repository. Production
N100 deployment and hardware-in-container acceptance remain open.
