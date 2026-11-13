# Playback capability reports

`docs/openapi.json` owns the native playback request contract. Automatic mode
considers the source container, selected audio, video codec, video pixel format,
audio channel limits, HDR support and subtitle burn-in. Codec names alone do not prove every profile,
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
    "max_audio_channels": 2,
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
Codec profiles/levels, resolution/frame-rate limits, audio layout support,
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

## Audio channel capabilities

`MediaTrack.channels` reports the probed channel count (zero if unavailable).
Automatic direct playback requires the selected audio codec and a known positive
channel count within `Capabilities.max_audio_channels`. The optional limit is
1–8; omission or null uses two channels. The web client explicitly reports two.
Absent audio streams remain eligible for direct playback; an audio stream with
unknown channel count requires conversion. Legacy `CanPlayMP4` also defaults to
stereo. An alternate audio track still requires remux/transcode selection.

HLS audio uses AAC, with stereo output or mono when the reported limit is one.
A larger limit permits suitable source multichannel direct playback; HLS does
not currently preserve surround or provide passthrough. Manual direct mode is
an explicit override. Channel counts do not prove support for layouts, codec
profiles, sample rates, object audio or a physical device. When automatic mode needs HLS and a capability report is present, the
server requires H.264 support and, for newly encoded video, support for the
8-bit `yuv420p` output. Remux keeps the already checked source pixel format.
AAC support is required only if the source has audio. Incompatible reports
receive HTTP 422 before session allocation. Manual modes and legacy requests
without capability reports keep their previous fallback behavior. The report
does not negotiate HLS transport, codec profiles/levels or hardware decoding.

The software corpus generates AAC 5.1 with distinct front, center, surround and
LFE frequencies and a separate stereo track. Decoded mono/stereo assertions
check non-LFE contributions, left/right routing, balanced center, clipping,
track isolation, output codec/channel count and a seeked video conversion.
The ordinary downmix may omit LFE; no bass-management claim is made. An explicit
six-channel report exercises direct selection without certifying a device.

```sh
OVL_ACCELERATOR=cpu cargo test --locked --test api \
  real_surround_audio_negotiates_and_downmixes_selected_channels -- --ignored --nocapture
```

## Playable video and embedded artwork

Automatic, remux and transcode playback select the first video track that is
neither attached artwork nor a timed thumbnail. A file with only audio and
artwork returns HTTP 422 in those modes. Manual direct mode still serves the
original file. Probe responses expose the two FFprobe disposition flags.

HDR frame inspection and HLS video mappings use the selected stream index.
This avoids probing color metadata from a different stream than the one
encoded. This does not add alternate-angle selection or audio library support.

## Music library preview

Create a Music library to scan MP3, M4A, FLAC, Ogg, Opus and WAV paths.
The scanner uses filenames and existing NFO import for browse titles; it
does not interpret episode-like names as TV episodes. Opening a track reads
a bounded subset of embedded tags (title, artist, album, album artist,
track and disc), including stream-level audio tags when container tags
are absent. A bounded scan also indexes tags for the Music albums view.

The web player uses native audio controls and shares progress and queues
with video. Automatic music playback checks the actual audio container,
codec and channel count against the client report. Without a capability
report, it accepts known mono/stereo MP3 only. If direct playback is
unsupported, a client advertising AAC and HLS can receive an audio-only
AAC HLS session. Conversion uses the first audio track and outputs one
or two channels according to the reported limit. It shares stream
admission, storage, cancellation and current-grant checks with video.
Explicit direct mode checks for audio and allows embedded artwork.
Music remux remains unavailable.

API clients may report `max_audio_sample_rate` from 8,000 to 384,000 Hz.
Automatic direct playback then requires a known source rate within the
ceiling. AAC output selects an encoder-supported rate at or below that
ceiling. Omitting the field preserves the previous negotiation and
encoding behavior. The web player leaves the rate unset because its
codec checks do not establish a device-wide sample-rate ceiling.

Music classification and tag indexing use additive tables and preserve the v2 library
rows, IDs and legacy kind constraint. Older binaries can preserve the
database but see these libraries as movies and do not offer this music
workflow. Audio conversion, standalone artist browsing and broader artwork and lyrics
support remain open.

### Local lyrics

The music player can open a UTF-8 sibling `.lrc` or `.txt` file, preferring
LRC. Timed LRC lines support multiple leading timestamps and seek the
track when selected; plain text stays untimed. The player renders lyrics
as text. The last valid signed LRC offset shifts every timestamp in milliseconds;
a positive value delays lyrics and a negative value advances them, clamped
at zero. Malformed offsets or offsets beyond 24 hours are ignored. Enhanced word timing is not supported.

The endpoint checks current grants and opens the stored music source
and sibling without following symlinks in any path component on Unix.
It limits input and expanded text to 128 KiB, 4,096 lines and 512
characters per line. An admitted filesystem worker retains its inspection
permit even after a disconnected client or a five-second HTTP deadline.
Shutdown waits for tracked work; kernel-blocked filesystem I/O still
depends on mount timeouts.

When there is no usable sidecar, MP3, FLAC and Ogg (Vorbis or Opus) can
display embedded text from FFprobe's `lyrics` or `unsyncedlyrics` fields.
Timed text uses the same LRC parser. The probe reads the already-open
source on stdin, with a pipe-only protocol whitelist, three permitted
demuxers, a five-second deadline and a 1 MiB output cap. That is an output
limit, not a source byte-read limit. The inspection permit remains held
through process cleanup. Ordinary media probes do not expose this lyric
text. Embedded M4A/WAV lyrics, binary synchronized tags and enhanced word
timing remain unsupported.

### Local album covers

Music cards use `cover.jpg`, `cover.png` or `cover.webp` from the track
folder, then the same extensions for `folder`. The existing artwork
cache is checked first. Artwork must resolve to the expected path, be
a regular file smaller than 20 MiB and is served with an explicit image
media type. Current media grants and library-root checks apply.
Artist-level artwork remains open. The route still
uses pathname opens after validation and does not claim protection
against all concurrent replacement races.

The open music player opts into embedded MP3/FLAC artwork via
`?embedded=true` when there is no cached or local cover. Browsing cards do
not opt in. The worker opens the source through no-follow directory
handles and supplies the regular file on stdin to FFmpeg. It restricts
input demuxers and protocols, uses single-threaded decoding/filtering,
sets a decoder pixel limit and emits one aspect-preserving PNG no larger
than 512×512. Extraction has a five-second deadline, 2 MiB output limit
and retains the inspection permit through process cleanup. The filesystem
open has its own deadline and holds the permit until it returns. No image
is written to the media or cache. Embedded artwork in other formats and
artist-level images remain open.

### Music artists and albums

The Music albums view groups indexed tracks by library, album artist
(or track artist when absent) and album title. Select an album to see
tracks sorted by numeric disc and track, then stored title and ID.
The album track view prefers embedded title tags; ordinary library
titles and manual metadata are not overwritten. Album search matches
album or artist text. Groups and tracks are paged in sets of 100 and
filtered by current grants. The Music artists view groups indexed album
artists (falling back to track artist) by library, counts albums and tracks,
and opens either matching albums or all indexed tracks for the artist.

A music scan admits up to 256 tag attempts per library during its
first 60 seconds. Each probe opens a regular file through no-follow
directory handles and uses a pipe-only protocol and the expected
demuxer. It holds the inspection permit through cleanup. The last
admitted probe can finish after the admission window. Unchanged file
fingerprints reuse tags; busy or timed-out probes can retry on a later
scan. Rows with mismatched recorded fingerprints are hidden. Larger
libraries may need repeated scans. Seek-dependent M4A metadata layouts,
artist artwork, separate contributor browsing and large-library performance remain open.
