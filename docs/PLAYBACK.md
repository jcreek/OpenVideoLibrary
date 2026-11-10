# Playback capability reports

`docs/openapi.json` owns the native playback request contract. Automatic mode
considers the source container, selected audio, video codec, video pixel format,
HDR support and subtitle burn-in. Codec names alone do not prove every profile,
bit depth or chroma format is supported.

Clients can send `Capabilities.video_pixel_formats`, an optional map from FFprobe
codec name to supported FFprobe pixel formats, for example:

```json
{
  "Mode": "auto",
  "Capabilities": {
    "containers": ["mp4", "webm"],
    "video_codecs": ["h264", "vp9"],
    "audio_codecs": ["aac", "opus"],
    "video_pixel_formats": {
      "h264": ["yuv420p", "yuvj420p"],
      "vp9": ["yuv420p"]
    },
    "hdr": false
  }
}
```

The Svelte client uses conservative 8-bit 4:2:0 reports. An older client that
omits the map, or a codec entry, receives the same conservative baseline for
H.264, HEVC, VP9 and AV1. Other source formats require transcoding unless the
client explicitly reports them. An empty list denies copying that codec's source
formats. Unknown source pixel formats require transcoding even if an empty string
appears in a report. Legacy `CanPlayMP4` requests also use the conservative
H.264 baseline for automatic direct/remux decisions.

An explicit additional format permits copying only when the remaining checks
allow it. It does not enable HDR without `hdr`, advertise an unsupported codec,
or bypass permissions. Manual `direct`, `remux` and `transcode` modes remain
explicit overrides; this capability selection applies to `auto`.

The generated software corpus includes ordinary 8-bit 4:2:0, 10-bit 4:2:0 and
8-bit 4:4:4 H.264. The latter two require conversion for conservative clients.
Tests inspect converted output as H.264 `yuv420p`, compare decoded source/output
RGB and check audio. These synthetic checks do not certify every file or device.
Codec profiles/levels, resolution/frame-rate limits, audio channel/layout support,
Dolby Vision, subtitle styling/fonts/bitmap formats and device-specific decoding
still need broader negotiation and acceptance evidence. Hardware acceptance and
performance remain separate; see `HDR.md`, `BENCHMARKS.md` and `PARITY.md`.

## Bitmap subtitle burn-in

The `Subtitle` request field selects a subtitle track by its zero-based subtitle
ordinal, as exposed by the player selector. Automatic mode chooses software
transcoding for burn-in. PGS, DVD, DVB and XSUB codec names now route to bitmap
composition rather than the text-subtitle filter. Hardware encoding remains
excluded for selected subtitles in this preview. Media access and session checks
use the existing current permissions and resolved source paths.

The verified generated PGS slice includes two tracks, opaque and half-transparent
white cues, explicit clear events, starts before/during/after a cue, single HLS
and adaptive 720p/480p output, audio and deletion cleanup. A tagged PQ black-base
fixture also checks that white cues retain their brightness after software SDR
conversion and that output is BT.709 `yuv420p`. This is not broad HDR imagery or
font/style acceptance. DVD, DVB and XSUB routing has no accepted real fixture yet.

Reproduce the software slice with:

```sh
OVL_ACCELERATOR=cpu cargo test --locked --test api \
  real_bitmap_subtitles_select_seek_clear_and_scale -- --ignored --nocapture
```

The original synthetic PGS display sets are generated in the Rust test rather
than downloaded. Their framing was checked against the
[FFmpeg PGS decoder](https://github.com/FFmpeg/FFmpeg/blob/n7.1/libavcodec/pgssubdec.c).
No film frames, subtitle artwork or external font files are included. These
Unix preview tests remain distinct from physical GPU and real-device acceptance.
