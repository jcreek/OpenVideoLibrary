use crate::{
    App,
    auth::User,
    error::{Error, Result},
    media,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use serde::Serialize;
use std::io::Read;
use std::{
    sync::{Arc, LazyLock},
    time::Duration,
};

const MAX_BYTES: usize = 128 * 1024;
const MAX_LINES: usize = 4096;
static STAMP: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"^\[([0-9]{1,3}):([0-5][0-9])(?:\.([0-9]{1,3}))?\]").unwrap()
});

#[derive(Serialize)]
pub struct LyricLine {
    time_ms: Option<u64>,
    text: String,
}

#[derive(Serialize)]
pub struct Lyrics {
    lines: Vec<LyricLine>,
}

fn parse(input: &str, timed: bool) -> Result<Lyrics> {
    let mut lines = Vec::new();
    let mut expanded_bytes = 0;
    let mut offset_ms = 0_i32;
    let input = input.trim_start_matches('\u{feff}');
    for line in input.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let mut rest = line;
        let mut stamps = Vec::new();
        if timed {
            if let Some(value) = line
                .strip_prefix("[offset:")
                .and_then(|s| s.strip_suffix(']'))
            {
                if let Ok(value) = value.trim().parse::<i32>()
                    && value.unsigned_abs() <= 86_400_000
                {
                    offset_ms = value;
                }
                continue;
            }
            while let Some(c) = STAMP.captures(rest) {
                if stamps.len() + lines.len() >= MAX_LINES {
                    return Err(Error::new(422, "too many lyric lines"));
                }
                let min: u64 = c[1].parse().unwrap();
                let sec: u64 = c[2].parse().unwrap();
                let fraction = c.get(3).map_or(0, |n| {
                    n.as_str().parse::<u64>().unwrap() * 10_u64.pow(3 - n.len() as u32)
                });
                stamps.push(min * 60_000 + sec * 1000 + fraction);
                rest = &rest[c.get(0).unwrap().end()..];
            }
            if stamps.is_empty()
                && ["[ar:", "[ti:", "[al:", "[by:", "[offset:", "[length:"]
                    .iter()
                    .any(|prefix| line.starts_with(prefix))
            {
                continue;
            }
        }
        let text = rest.trim();
        if text.is_empty() {
            continue;
        }
        if lines.len() + stamps.len().max(1) > MAX_LINES {
            return Err(Error::new(422, "too many lyric lines"));
        }
        // Cap expanded text: one input line can have many timestamps.
        let text: String = text.chars().take(512).collect();
        expanded_bytes += text.len() * stamps.len().max(1);
        if expanded_bytes > MAX_BYTES {
            return Err(Error::new(422, "expanded lyrics exceed size limit"));
        }
        if stamps.is_empty() {
            lines.push(LyricLine {
                time_ms: None,
                text,
            });
        } else {
            lines.extend(stamps.into_iter().map(|time_ms| LyricLine {
                time_ms: Some(time_ms),
                text: text.clone(),
            }));
        }
    }
    // Apply the last valid global offset even when it follows the cues.
    for line in &mut lines {
        if let Some(time) = &mut line.time_ms {
            *time = time.saturating_add_signed(i64::from(offset_ms));
        }
    }
    lines.sort_by_key(|l| l.time_ms.unwrap_or(u64::MAX));
    Ok(Lyrics { lines })
}

fn read_lyrics(path: &std::path::Path) -> Result<Option<Lyrics>> {
    for extension in ["lrc", "txt"] {
        let Ok(file) = media::open_registered_file(&path.with_extension(extension)) else {
            continue;
        };
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            continue;
        }
        if metadata.len() > MAX_BYTES as u64 {
            return Err(Error::new(422, "lyrics exceed size limit"));
        }
        let mut bytes = Vec::new();
        file.take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes)?;
        if bytes.len() > MAX_BYTES {
            return Err(Error::new(422, "lyrics exceed size limit"));
        }
        let text =
            std::str::from_utf8(&bytes).map_err(|_| Error::new(422, "lyrics must be UTF-8"))?;
        return parse(text, extension == "lrc").map(Some);
    }
    Ok(None)
}

pub async fn get(
    State(app): State<Arc<App>>,
    Extension(user): Extension<User>,
    Path(id): Path<i64>,
) -> Result<Json<Lyrics>> {
    let (path, root, _, kind) = media::locate_item(&app, user, id, true).await?;
    if kind != "music" {
        return Err(Error::new(404, "lyrics unavailable"));
    }
    let slot = app
        .inspection
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::new(429, "media inspection busy"))?;
    // Timeout or disconnect stops waiting, not the underlying filesystem I/O.
    let work = app.tasks.spawn_blocking(move || {
        let unavailable = |_| Error::new(404, "media unavailable");
        let real = std::path::PathBuf::from(path);
        if !real.starts_with(root) {
            return Err(Error::new(404, "media unavailable"));
        }
        let file = media::open_registered_file(&real).map_err(unavailable)?;
        if !file.metadata().map_err(unavailable)?.is_file() {
            return Err(Error::new(404, "media unavailable"));
        }
        let embedded = real.extension().and_then(|v| v.to_str()).is_some_and(|v| {
            ["mp3", "flac", "ogg", "opus"]
                .iter()
                .any(|kind| v.eq_ignore_ascii_case(kind))
        });
        let sidecar = read_lyrics(&real)?;
        Ok((sidecar, embedded, file, slot))
    });
    let (sidecar, embedded, file, slot) = tokio::time::timeout(Duration::from_secs(5), work)
        .await
        .map_err(|_| Error::new(504, "lyrics read timed out"))?
        .map_err(|_| Error::new(500, "lyrics worker failed"))??;
    let lyrics = if let Some(sidecar) = sidecar {
        sidecar
    } else if embedded {
        let data = media::probe_lyrics(&app, file, slot).await?;
        let info: serde_json::Value = serde_json::from_slice(&data)
            .map_err(|_| Error::new(503, "invalid lyrics probe response"))?;
        let tags = std::iter::once(&info["format"]["tags"]).chain(
            info["streams"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|s| s["codec_type"] == "audio")
                .map(|s| &s["tags"]),
        );
        let mut text = None;
        for group in tags.filter_map(serde_json::Value::as_object) {
            if let Some(value) = group.iter().find_map(|(key, value)| {
                ["lyrics", "unsyncedlyrics"]
                    .iter()
                    .any(|k| key.eq_ignore_ascii_case(k))
                    .then(|| value.as_str())
                    .flatten()
                    .filter(|s| !s.trim().is_empty())
            }) {
                text = Some(value);
                break;
            }
        }
        if let Some(text) = text {
            if text.len() > MAX_BYTES {
                return Err(Error::new(422, "lyrics exceed size limit"));
            }
            parse(text, true)?
        } else {
            Lyrics { lines: Vec::new() }
        }
    } else {
        Lyrics { lines: Vec::new() }
    };
    Ok(Json(lyrics))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lrc_offset_moves_all_cues_and_clamps_at_zero() {
        for (tag, first, second) in [("+500", 600, 2500), ("-500", 0, 1500)] {
            let input = format!(
                "[00:02]Second\n[00:00.10]First\n[offset:{tag}]\n[offset:999999999999]\nPlain"
            );
            let lyrics = parse(&input, true).unwrap();
            assert_eq!(
                lyrics
                    .lines
                    .iter()
                    .map(|line| line.time_ms)
                    .collect::<Vec<_>>(),
                vec![Some(first), Some(second), None]
            );
            assert_eq!(lyrics.lines[0].text, "First");
            assert_eq!(lyrics.lines[2].text, "Plain");
        }
        let plain = parse("[offset:+500]", false).unwrap();
        assert_eq!(plain.lines[0].time_ms, None);
        assert_eq!(plain.lines[0].text, "[offset:+500]");
    }
}
