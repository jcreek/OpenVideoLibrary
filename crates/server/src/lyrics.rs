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
    let input = input.trim_start_matches('\u{feff}');
    for line in input.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let mut rest = line;
        let mut stamps = Vec::new();
        if timed {
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
    lines.sort_by_key(|l| l.time_ms.unwrap_or(u64::MAX));
    Ok(Lyrics { lines })
}

#[cfg(unix)]
fn open_sibling(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    use std::{
        ffi::CString,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::ffi::OsStrExt,
        },
        path::Component,
    };
    let mut parts = path.components().peekable();
    if parts.next() != Some(Component::RootDir) {
        return Err(std::io::Error::other("expected absolute lyrics path"));
    }
    let mut directory = std::fs::File::open("/")?;
    while let Some(Component::Normal(name)) = parts.next() {
        let name = CString::new(name.as_bytes())?;
        let flags = libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK;
        let flags = if parts.peek().is_some() {
            flags | libc::O_DIRECTORY
        } else {
            flags
        };
        // The directory handle anchors each component. No symlink is followed.
        let fd = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        directory = unsafe { std::fs::File::from_raw_fd(fd) };
        if parts.peek().is_none() {
            return Ok(directory);
        }
    }
    Err(std::io::Error::other("invalid lyrics path"))
}

#[cfg(not(unix))]
fn open_sibling(_: &std::path::Path) -> std::io::Result<std::fs::File> {
    Err(std::io::Error::other("local lyrics require Unix"))
}

fn read_lyrics(path: &std::path::Path) -> Result<Lyrics> {
    for extension in ["lrc", "txt"] {
        let Ok(file) = open_sibling(&path.with_extension(extension)) else {
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
        return parse(text, extension == "lrc");
    }
    Ok(Lyrics { lines: Vec::new() })
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
        let _slot = slot;
        let unavailable = |_| Error::new(404, "media unavailable");
        // New music libraries persist a canonical root, and scanning only
        // records regular files under it. Do not re-resolve a replaced root.
        let real = std::path::PathBuf::from(path);
        if !real.starts_with(root)
            || !open_sibling(&real)
                .map_err(unavailable)?
                .metadata()
                .map_err(unavailable)?
                .is_file()
        {
            return Err(Error::new(404, "media unavailable"));
        }
        read_lyrics(&real)
    });
    let lyrics = tokio::time::timeout(Duration::from_secs(5), work)
        .await
        .map_err(|_| Error::new(504, "lyrics read timed out"))?
        .map_err(|_| Error::new(500, "lyrics worker failed"))??;
    Ok(Json(lyrics))
}
