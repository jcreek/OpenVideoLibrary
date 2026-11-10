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
