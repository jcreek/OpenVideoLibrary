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

The generated native PGS, DVD, DVB and XSUB corpus checks 70 application
sessions across two tracks, starts before/during/after cues, single HLS and
adaptive 720p/480p output, decoded picture/audio, invalid selection and deletion
cleanup. PGS/DVD/DVB cover SDR and tagged PQ black-base sources with BT.709
`yuv420p` output; XSUB covers SDR only. Native controls establish cue visibility,
clearing and alpha behavior before application playback. The XSUB DXSB-style
fixture has opaque cues; extended alpha/DXSA remains unverified. This is generated
media acceptance, not broad HDR imagery, language or real-device acceptance.

Frequent keyframes exposed a lost-cue bug after input seeking. Indexed Matroska
and AVI now use a second input from the same resolved source to preroll subtitle
decoding, with subtitle timestamps shifted by the seek offset and composition
before rendition scaling. Other second-input streams are unmapped beyond initial
probing. MPEG-TS uses sequential decoding from the source origin and output-side
trimming because independent input timestamp origins did not preserve timing.
These approaches retain active cues and clear them correctly. Subtitle preroll
still reads earlier container data; long-file/SMB seek I/O and latency are not
accepted, especially for sequential MPEG-TS.

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

## Styled text subtitles and source time

Embedded text tracks offer both browser text and software burn-in choices. For
ASS/SSA, burn-in is the choice intended to preserve source styling. Browser text
conversion does not preserve embedded font geometry or every source effect.
External sibling SRT remains browser text in this preview.

Burn-in restores the source cue timeline around text rendering, accounting for
both the requested seek and `MediaInfo.format.start_time`, then restores the
playback timeline before encoding. The additive probe field is a raw source
origin string, empty when unavailable. It is distinct from playback-relative
`Start`; the existing raw duration field is unchanged.

The generated ASS slice embeds an original 1,240-byte font with a deliberately
unusual glyph, plus a fontless control that must fail to match it. Tests sample
red glyph geometry and position, cue clearing and timed audio at starts before,
during and after a cue, for zero- and five-second source origins. The generator
is `scripts/generate-font-fixture.py`; fontTools is a regeneration-only tool and
is not a server/test dependency. No system font is copied or installed.

This slice does not certify SSA, every font/language, animated effects, karaoke,
VFR, timestamp discontinuities, delayed tracks or external-subtitle burn-in.
Source-duration interpretation across nonzero-origin containers also needs a
broader corpus; this fix concerns cue alignment and does not redefine duration.
Physical GPU subtitle rendering remains outside the software acceptance.
