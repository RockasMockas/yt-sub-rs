use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};
use eyre::Result;
use home::home_dir;
use serde::{Deserialize, Serialize};
use yt_sub_core::video::Video;

/// A seen video plus the timestamp of when it was first recorded.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CacheEntry {
    pub video: Video,
    pub seen_at: DateTime<Utc>,
}

/// OS-dependent appdata directory for yt-sub-rs.
fn appdata_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        if let Ok(p) = std::env::var("LOCALAPPDATA") {
            return PathBuf::from(p);
        }
        if let Ok(p) = std::env::var("APPDATA") {
            return PathBuf::from(p);
        }
        home_dir().unwrap_or_else(|| PathBuf::from("."))
    }
    #[cfg(target_os = "macos")]
    {
        home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Library/Application Support")
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(p) = std::env::var("XDG_DATA_HOME") {
            return PathBuf::from(p);
        }
        home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".local/share")
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        home_dir().unwrap_or_else(|| PathBuf::from("."))
    }
}

/// Full path to the persistent seen-videos JSON cache.
pub fn cache_path() -> PathBuf {
    appdata_dir().join("yt-sub-rs").join("seen_videos.json")
}

/// Load the cache from `path`. Returns empty on missing or corrupt file.
pub fn load_from(path: &Path) -> Vec<CacheEntry> {
    match fs::read_to_string(path) {
        Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
        Err(_) => vec![],
    }
}

/// Load the cache from the default OS appdata path.
pub fn load() -> Vec<CacheEntry> {
    load_from(&cache_path())
}

/// Merge `fetched` videos into `cache`, deduplicating by video link.
/// Returns only the newly-added entries (pushed with `seen_at = now`).
pub fn merge(
    cache: &mut Vec<CacheEntry>,
    fetched: Vec<Video>,
    now: DateTime<Utc>,
) -> Vec<CacheEntry> {
    let mut seen: HashSet<String> = cache.iter().map(|e| e.video.link.clone()).collect();
    let mut newly_seen = vec![];

    for video in fetched {
        if seen.insert(video.link.clone()) {
            let entry = CacheEntry {
                video,
                seen_at: now,
            };
            newly_seen.push(entry.clone());
            cache.push(entry);
        }
    }

    newly_seen
}

/// Persist `cache` atomically (temp-file + rename) to `path`.
pub fn save_to(path: &Path, cache: &[CacheEntry]) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }

    let json = serde_json::to_string_pretty(cache)?;
    let tmp = path.with_extension("json.tmp");

    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&tmp)?;
    file.write_all(json.as_bytes())?;
    file.flush()?;

    fs::rename(&tmp, path)?;
    Ok(())
}

/// Persist `cache` to the default OS appdata path.
pub fn save(cache: &[CacheEntry]) -> Result<()> {
    save_to(&cache_path(), cache)
}

/// Return cache entries published within the last `hours`.
pub fn window(cache: &[CacheEntry], hours: u16) -> Vec<CacheEntry> {
    let cutoff = Utc::now() - Duration::hours(hours as i64);
    cache
        .iter()
        .filter(|e| e.video.published_at > cutoff)
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;

    fn video(link: &str, published: &str) -> Video {
        Video {
            channel: "Test Channel".to_string(),
            channel_handle: Some("@TestChannel".to_string()),
            title: format!("Title {}", link),
            link: link.to_string(),
            published_at: DateTime::parse_from_rfc3339(published)
                .unwrap()
                .with_timezone(&Utc),
        }
    }

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("yt-sub-cache-test-{}-{}", name, std::process::id()))
    }

    #[test]
    fn load_missing_returns_empty() {
        let path = temp_path("missing").with_extension("json");
        let _ = fs::remove_file(&path);
        assert!(load_from(&path).is_empty());
    }

    #[test]
    fn load_corrupt_returns_empty() {
        let path = temp_path("corrupt").with_extension("json");
        fs::write(&path, b"{ not valid json ]]").unwrap();
        assert!(load_from(&path).is_empty());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn merge_dedups_by_link_and_returns_only_new() {
        let now = Utc::now();
        let mut cache: Vec<CacheEntry> = vec![];

        let newly = merge(
            &mut cache,
            vec![
                video("a", "2024-01-01T10:00:00Z"),
                video("a", "2024-01-01T10:00:00Z"),
            ],
            now,
        );
        assert_eq!(newly.len(), 1);
        assert_eq!(cache.len(), 1);

        // Second merge: same link + one new link -> only the new one returned.
        let newly2 = merge(
            &mut cache,
            vec![
                video("a", "2024-01-01T10:00:00Z"),
                video("b", "2024-01-02T10:00:00Z"),
            ],
            now,
        );
        assert_eq!(newly2.len(), 1);
        assert_eq!(newly2[0].video.link, "b");
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn save_then_load_round_trip() {
        let path = temp_path("roundtrip").with_extension("json");
        let now = Utc::now();
        let cache = vec![CacheEntry {
            video: video("x", "2024-01-01T10:00:00Z"),
            seen_at: now,
        }];

        save_to(&path, &cache).unwrap();
        let loaded = load_from(&path);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].video.link, "x");
        assert_eq!(loaded[0].seen_at, now);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn save_is_byte_identical_when_rewritten_same() {
        let path = temp_path("identical").with_extension("json");
        let now = Utc::now();
        let cache = vec![CacheEntry {
            video: video("y", "2024-01-01T10:00:00Z"),
            seen_at: now,
        }];

        save_to(&path, &cache).unwrap();
        let first = fs::read(&path).unwrap();
        save_to(&path, &cache).unwrap();
        let second = fs::read(&path).unwrap();
        assert_eq!(first, second);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn fetch_failure_keeps_cache_and_serves_window() {
        // Seed cache with one recent and one old video, persisted.
        let path = temp_path("fetchfail").with_extension("json");
        let now = Utc::now();
        let recent = (now - Duration::minutes(30)).to_rfc3339();
        let old = (now - Duration::days(10)).to_rfc3339();

        let mut cache = vec![
            CacheEntry {
                video: video("recent", &recent),
                seen_at: now,
            },
            CacheEntry {
                video: video("old", &old),
                seen_at: now,
            },
        ];
        save_to(&path, &cache).unwrap();
        let bytes_before = fs::read(&path).unwrap();

        // Simulate a run where every channel fetch fails: nothing fetched.
        let fetched: Vec<Video> = vec![];
        let cache_merge = merge(&mut cache, fetched, now);

        // AC3: no changes -> run.rs skips save -> file byte-identical.
        assert!(cache_merge.is_empty());
        assert_eq!(fs::read(&path).unwrap(), bytes_before);

        // AC4: hours-offset still returns cached recent video.
        let win = window(&cache, 2);
        assert_eq!(win.len(), 1);
        assert_eq!(win[0].video.link, "recent");

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn no_flag_scopes_whole_cache_to_last_run_at() {
        // Non-flag notify set = whole cache filtered by last_run_at,
        // matching the original notify-new watermark semantics.
        let now = Utc::now();
        let last_run_at = now - Duration::days(7);
        let recent = (now - Duration::days(1)).to_rfc3339();
        let old = (now - Duration::days(30)).to_rfc3339();

        let cache: Vec<CacheEntry> = vec![
            CacheEntry {
                video: video("recent", &recent),
                seen_at: now,
            },
            CacheEntry {
                video: video("old", &old),
                seen_at: now,
            },
        ];

        let notify: Vec<_> = cache
            .iter()
            .filter(|e| e.video.published_at > last_run_at)
            .collect();
        assert_eq!(notify.len(), 1);
        assert_eq!(notify[0].video.link, "recent");
    }

    #[test]
    fn cached_but_unnotified_video_is_not_dropped() {
        // Regression: a prior hours_offset run can cache a video outside its
        // window without advancing last_run_at (empty output => no touch). A
        // later non-flag run must still notify it, so scoping the notify set
        // to only this-run's cache_merge would silently drop it.
        let now = Utc::now();
        let last_run_at = now - Duration::days(7);
        let published = (now - Duration::days(1)).to_rfc3339();
        let v = video("dropped", &published);

        // Run A (hours_offset): video cached, window empty => last_run_at unchanged.
        let mut cache: Vec<CacheEntry> = vec![];
        let merge_a = merge(&mut cache, vec![v.clone()], now);
        assert_eq!(merge_a.len(), 1);
        assert!(window(&cache, 1).is_empty());

        // Run B (no flag): cache_merge is empty (already cached), but the
        // whole-cache filter still surfaces it for notification.
        let merge_b = merge(&mut cache, vec![v.clone()], now);
        assert!(merge_b.is_empty());
        let notify: Vec<_> = cache
            .iter()
            .filter(|e| e.video.published_at > last_run_at)
            .map(|e| e.video.link.clone())
            .collect();
        assert_eq!(notify, vec!["dropped".to_string()]);
    }

    #[test]
    fn window_filters_by_hours() {
        let now = Utc::now();
        let recent = (now - Duration::minutes(30)).to_rfc3339();
        let old = (now - Duration::days(10)).to_rfc3339();

        let cache = vec![
            CacheEntry {
                video: video("recent", &recent),
                seen_at: now,
            },
            CacheEntry {
                video: video("old", &old),
                seen_at: now,
            },
        ];

        let win = window(&cache, 2);
        assert_eq!(win.len(), 1);
        assert_eq!(win[0].video.link, "recent");
    }
}
