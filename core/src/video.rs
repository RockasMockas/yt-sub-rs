use chrono::{DateTime, Utc};
use eyre::Result;
use log::{debug, warn};
use serde::{Deserialize, Serialize};
use xmltojson::to_json;

use crate::notifier::Notifier;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Video {
    pub channel: String,
    pub channel_handle: Option<String>,
    pub title: String,
    pub link: String,
    pub published_at: DateTime<Utc>,
}

impl Video {
    pub fn parse_rss(rss_data: String, channel_handle: Option<String>) -> Result<Vec<Video>> {
        let mut videos = vec![];
        let json = to_json(&rss_data).expect("Failed to convert XML to JSON");
        let channel = json["feed"]["author"]["name"]
            .as_str()
            .ok_or_else(|| eyre::eyre!("RSS feed missing author name"))?;

        // Handle case where channel has no videos (no "entry" field)
        let videos_data = match json["feed"]["entry"].as_array() {
            Some(data) => data,
            None => return Ok(videos), // Return empty vector if no videos
        };

        for video_data in videos_data {
            match Self::parse_single_video(video_data, channel, channel_handle.as_deref()) {
                Ok(video) => {
                    // Shorts are dropped at the source so they never reach the
                    // cache, the outputs, or the notifiers.
                    if video.is_short() {
                        debug!("Skipping YouTube Short: {}", video.link);
                    } else {
                        videos.push(video);
                    }
                }
                Err(e) => {
                    warn!("Skipping malformed video entry: {e}");
                }
            }
        }

        Ok(videos)
    }

    fn parse_single_video(
        video_data: &serde_json::Value,
        channel: &str,
        channel_handle: Option<&str>,
    ) -> Result<Video> {
        let title = video_data["title"]
            .as_str()
            .ok_or_else(|| eyre::eyre!("Missing video title"))?;
        let published_at = video_data["published"]
            .as_str()
            .ok_or_else(|| eyre::eyre!("Missing published date"))?;
        let published_at: DateTime<Utc> = published_at
            .parse()
            .map_err(|e| eyre::eyre!("Invalid date format: {e}"))?;
        let link = video_data["link"]["@href"]
            .as_str()
            .ok_or_else(|| eyre::eyre!("Missing video link"))?;

        Ok(Video {
            channel: channel.to_string(),
            channel_handle: channel_handle.map(String::from),
            title: title.to_string(),
            link: link.to_string(),
            published_at,
        })
    }

    /// Whether this video points at a YouTube Short.
    pub fn is_short(&self) -> bool {
        is_short_link(&self.link)
    }

    pub fn notification_text(&self, notifier: &Notifier) -> String {
        let time_ago = self.format_time_ago();
        let channel_handle = self.channel_handle.as_deref().unwrap_or("");

        match notifier {
            Notifier::Log() => {
                let mut parts = vec![];

                // Start with dash
                parts.push("-".to_string());

                // Add channel handle if available
                if !channel_handle.is_empty() {
                    parts.push(channel_handle.to_string());
                }

                // Add title and link in markdown format
                parts.push(format!("[{}]({})", self.title, self.link));

                // Add time if available
                if !time_ago.is_empty() {
                    parts.push(format!("- {}", time_ago));
                }

                parts.join(" ")
            }
            Notifier::Slack(_) => {
                format!(
                    "*New video - {}* <{}|{}>",
                    self.channel, self.link, self.title
                )
            }
            Notifier::Telegram => {
                todo!()
            }
        }
    }

    fn format_time_ago(&self) -> String {
        let now = Utc::now();
        let duration = now.signed_duration_since(self.published_at);

        if duration.num_hours() > 0 {
            let hours = duration.num_hours();
            if hours == 1 {
                "1 hour ago".to_string()
            } else {
                format!("{} hours ago", hours)
            }
        } else if duration.num_minutes() > 0 {
            let minutes = duration.num_minutes();
            if minutes == 1 {
                "1 minute ago".to_string()
            } else {
                format!("{} minutes ago", minutes)
            }
        } else {
            "just now".to_string()
        }
    }
}

/// Shorts are served from their own path segment
/// (`https://www.youtube.com/shorts/<video_id>`, on any host, optionally with a
/// query string), while regular videos use `watch`, `live`, `embed`, etc.
pub fn is_short_link(link: &str) -> bool {
    let after_authority = link.split_once("://").map_or(link, |(_, rest)| rest);
    let path = after_authority
        .split(['?', '#'])
        .next()
        .unwrap_or(after_authority);

    let mut segments = path.split('/').filter(|s| !s.is_empty());
    // A scheme-less link still starts with its host, which isn't a path segment.
    let first_segment = match segments.next() {
        Some(host) if host.contains('.') => segments.next(),
        other => other,
    };

    first_segment.is_some_and(|segment| segment.eq_ignore_ascii_case("shorts"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[tokio::test]
    async fn parse_videos_test() {
        let rss_data = fs::read_to_string("src/fixtures/yt_videos_data.xml").unwrap();
        let videos = Video::parse_rss(rss_data, Some("@TestChannel".to_string())).unwrap();
        assert_eq!(videos.len(), 15);
    }

    #[tokio::test]
    async fn parse_empty_channel_test() {
        let rss_data = fs::read_to_string("src/fixtures/empty_channel.xml").unwrap();
        let videos = Video::parse_rss(rss_data, Some("@EmptyChannel".to_string())).unwrap();
        assert_eq!(videos.len(), 0);
    }

    /// Minimal Atom feed with one regular video and two Shorts. Needs 2+ entries,
    /// because xmltojson collapses a lone `<entry>` into an object instead of an
    /// array.
    const SHORTS_FEED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
 <author><name>Shorts Channel</name></author>
 <entry>
  <title>A regular video</title>
  <link rel="alternate" href="https://www.youtube.com/watch?v=regular000001"/>
  <published>2024-08-22T16:08:11+00:00</published>
 </entry>
 <entry>
  <title>A short</title>
  <link rel="alternate" href="https://www.youtube.com/shorts/short00000001"/>
  <published>2024-08-22T15:08:11+00:00</published>
 </entry>
 <entry>
  <title>A mobile short with a query string</title>
  <link rel="alternate" href="https://m.youtube.com/shorts/short00000002?feature=share"/>
  <published>2024-08-22T14:08:11+00:00</published>
 </entry>
</feed>"#;

    #[tokio::test]
    async fn parse_rss_filters_out_shorts() {
        let videos =
            Video::parse_rss(SHORTS_FEED.to_string(), Some("@ShortsChannel".to_string())).unwrap();

        assert_eq!(videos.len(), 1);
        assert_eq!(
            videos[0].link,
            "https://www.youtube.com/watch?v=regular000001"
        );
    }

    #[test]
    fn test_is_short_link() {
        let shorts = [
            "https://www.youtube.com/shorts/abc123",
            "https://youtube.com/shorts/abc123",
            "http://m.youtube.com/shorts/abc123?feature=share",
            "https://www.youtube.com/shorts/abc123?si=x1",
            "https://www.youtube.com/shorts/abc123#t=1",
            "https://www.youtube.com/Shorts/abc123",
            "youtube.com/shorts/abc123",
        ];
        for link in shorts {
            assert!(is_short_link(link), "expected short: {link}");
        }

        let not_shorts = [
            "https://www.youtube.com/watch?v=abc123",
            "https://www.youtube.com/watch?v=abc123&t=1",
            "https://m.youtube.com/watch?v=abc123",
            "https://www.youtube.com/live/abc123",
            "https://www.youtube.com/embed/abc123",
            "https://www.youtube.com/playlist?list=abc123",
            "https://www.youtube.com/@SomeChannel",
            "https://www.youtube.com/channel/UCabc123",
            "https://www.youtube.com/feed/subscriptions",
        ];
        for link in not_shorts {
            assert!(!is_short_link(link), "unexpected short: {link}");
        }
    }

    #[test]
    fn test_video_is_short() {
        assert!(Video {
            channel: "Test Channel".to_string(),
            channel_handle: None,
            title: "Short".to_string(),
            link: "https://www.youtube.com/shorts/abc123".to_string(),
            published_at: DateTime::parse_from_rfc3339("2024-01-01T12:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
        }
        .is_short());
    }

    #[test]
    fn test_video_serde_round_trip() {
        use chrono::DateTime;

        let video = Video {
            channel: "Test Channel".to_string(),
            channel_handle: Some("@TestChannel".to_string()),
            title: "Test Video Title".to_string(),
            link: "https://www.youtube.com/watch?v=serde1".to_string(),
            published_at: DateTime::parse_from_rfc3339("2024-01-01T12:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
        };

        let json = serde_json::to_string(&video).unwrap();
        let back: Video = serde_json::from_str(&json).unwrap();

        assert_eq!(back.link, video.link);
        assert_eq!(back.title, video.title);
        assert_eq!(back.channel_handle, video.channel_handle);
        assert_eq!(back.published_at, video.published_at);
    }

    #[test]
    fn test_notification_text_format() {
        use crate::notifier::Notifier;

        let video = Video {
            channel: "Test Channel".to_string(),
            channel_handle: Some("@TestChannel".to_string()),
            title: "Test Video Title".to_string(),
            link: "https://www.youtube.com/watch?v=test123".to_string(),
            published_at: DateTime::parse_from_rfc3339("2024-01-01T12:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
        };

        let notifier = Notifier::Log();
        let result = video.notification_text(&notifier);

        // Should start with "- @TestChannel"
        assert!(result.starts_with("- @TestChannel"));
        // Should contain markdown link format
        assert!(result.contains("[Test Video Title](https://www.youtube.com/watch?v=test123)"));
        // Should not contain "New video -"
        assert!(!result.contains("New video -"));
        // Should follow the format: "- <channel_handle> [title](url) - X hours ago"
        assert!(result
            .contains("@TestChannel [Test Video Title](https://www.youtube.com/watch?v=test123)"));
    }

    #[test]
    fn test_video_sorting_by_published_date() {
        use chrono::DateTime;

        let mut videos = vec![
            Video {
                channel: "Channel A".to_string(),
                channel_handle: Some("@ChannelA".to_string()),
                title: "Older Video".to_string(),
                link: "https://www.youtube.com/watch?v=old".to_string(),
                published_at: DateTime::parse_from_rfc3339("2024-01-01T10:00:00Z")
                    .unwrap()
                    .with_timezone(&chrono::Utc),
            },
            Video {
                channel: "Channel B".to_string(),
                channel_handle: Some("@ChannelB".to_string()),
                title: "Newer Video".to_string(),
                link: "https://www.youtube.com/watch?v=new".to_string(),
                published_at: DateTime::parse_from_rfc3339("2024-01-01T14:00:00Z")
                    .unwrap()
                    .with_timezone(&chrono::Utc),
            },
            Video {
                channel: "Channel C".to_string(),
                channel_handle: Some("@ChannelC".to_string()),
                title: "Newest Video".to_string(),
                link: "https://www.youtube.com/watch?v=newest".to_string(),
                published_at: DateTime::parse_from_rfc3339("2024-01-01T16:00:00Z")
                    .unwrap()
                    .with_timezone(&chrono::Utc),
            },
        ];

        // Sort by publication date (newest first)
        videos.sort_by(|a, b| b.published_at.cmp(&a.published_at));

        // Verify sorting order
        assert_eq!(videos[0].title, "Newest Video");
        assert_eq!(videos[1].title, "Newer Video");
        assert_eq!(videos[2].title, "Older Video");
    }
}
