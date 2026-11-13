CREATE TABLE IF NOT EXISTS ovl_lists (
 id INTEGER PRIMARY KEY, owner_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 kind TEXT NOT NULL CHECK(kind IN ('collection','playlist','queue')),
 name TEXT NOT NULL CHECK(length(name) BETWEEN 1 AND 200),
 revision INTEGER NOT NULL DEFAULT 1, cursor INTEGER NOT NULL DEFAULT 0,
 updated INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ovl_lists_owner ON ovl_lists(owner_id,id);
CREATE TABLE IF NOT EXISTS ovl_list_entries (
 list_id INTEGER NOT NULL REFERENCES ovl_lists(id) ON DELETE CASCADE,
 position INTEGER NOT NULL CHECK(position>=0),
 item_id INTEGER NOT NULL REFERENCES items(id) ON DELETE CASCADE,
 PRIMARY KEY(list_id,position)
);
-- Keep the legacy movie/TV CHECK constraint and library IDs intact.
CREATE TABLE IF NOT EXISTS ovl_music_libraries (
 library_id INTEGER PRIMARY KEY REFERENCES libraries(id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS ovl_music_tags (
 file_id INTEGER PRIMARY KEY REFERENCES media_files(id) ON DELETE CASCADE,
 mtime INTEGER NOT NULL, size INTEGER NOT NULL,
 title TEXT NOT NULL, artist TEXT NOT NULL, album TEXT NOT NULL,
 disc INTEGER NOT NULL, track INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ovl_music_tags_album ON ovl_music_tags(album,artist,disc,track,file_id);
-- Failed tag probes for the file version they saw, so a scan's probe budget
-- reaches later tracks; each is retried at retry_at or when the file changes.
CREATE TABLE IF NOT EXISTS ovl_music_probe_failures (
 file_id INTEGER PRIMARY KEY REFERENCES media_files(id) ON DELETE CASCADE,
 mtime INTEGER NOT NULL, size INTEGER NOT NULL, retry_at INTEGER NOT NULL
);
