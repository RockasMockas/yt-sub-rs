use chrono::{DateTime, Utc};
use eyre::Result;
use xmltojson::to_json;

use crate::notifier::Notifier;

#[derive(Debug)]
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
        let channel = json["feed"]["author"]["name"].as_str().unwrap();

        // Handle case where channel has no videos (no "entry" field)
        let videos_data = match json["feed"]["entry"].as_array() {
            Some(data) => data,
            None => return Ok(videos), // Return empty vector if no videos
        };

        for video_data in videos_data {
            let title = video_data["title"].as_str().unwrap();
            let published_at = video_data["published"].as_str().unwrap();
            let published_at: DateTime<Utc> =
                published_at.parse().expect("Failed to parse DateTime");
            let link = video_data["link"]["@href"].as_str().unwrap();

            let video = Video {
                channel: channel.to_string(),
                channel_handle: channel_handle.clone(),
                title: title.to_string(),
                link: link.to_string(),
                published_at,
            };

            videos.push(video);
        }

        Ok(videos)
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

    #[test]
    fn test_notification_text_format() {
        use chrono::DateTime;
        use crate::notifier::Notifier;

        let video = Video {
            channel: "Test Channel".to_string(),
            channel_handle: Some("@TestChannel".to_string()),
            title: "Test Video Title".to_string(),
            link: "https://www.youtube.com/watch?v=test123".to_string(),
            published_at: DateTime::parse_from_rfc3339("2024-01-01T12:00:00Z").unwrap().with_timezone(&chrono::Utc),
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
        assert!(result.contains("@TestChannel [Test Video Title](https://www.youtube.com/watch?v=test123)"));
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
                published_at: DateTime::parse_from_rfc3339("2024-01-01T10:00:00Z").unwrap().with_timezone(&chrono::Utc),
            },
            Video {
                channel: "Channel B".to_string(),
                channel_handle: Some("@ChannelB".to_string()),
                title: "Newer Video".to_string(),
                link: "https://www.youtube.com/watch?v=new".to_string(),
                published_at: DateTime::parse_from_rfc3339("2024-01-01T14:00:00Z").unwrap().with_timezone(&chrono::Utc),
            },
            Video {
                channel: "Channel C".to_string(),
                channel_handle: Some("@ChannelC".to_string()),
                title: "Newest Video".to_string(),
                link: "https://www.youtube.com/watch?v=newest".to_string(),
                published_at: DateTime::parse_from_rfc3339("2024-01-01T16:00:00Z").unwrap().with_timezone(&chrono::Utc),
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
