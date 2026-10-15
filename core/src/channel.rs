use std::fmt::{self, Display, Formatter};

use chrono::{DateTime, Utc};
use eyre::{eyre, Result};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{user_settings::API_HOST, video::Video};

const RSS_HOST: &str = "https://www.youtube.com";
const YOUTUBE_HOST: &str = "https://www.youtube.com";
const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:120.0) Gecko/20100101 Firefox/120.0";
// Bypass the EU cookie-consent interstitial so the channel page is returned directly.
const CONSENT_COOKIE: &str = "CONSENT=YES+cb; SOCS=CAI";

#[derive(Debug, Deserialize, Serialize, PartialEq, Clone)]
pub struct Channel {
    pub handle: String,
    pub description: String,
    pub channel_id: String,
}

impl Display for Channel {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "name: {name}
handle: {handle}
channel_id: {channel_id}
channel_url: {channel_url}
RSS feed: {channel_feed}",
            name = self.description,
            handle = self.handle,
            channel_id = self.channel_id,
            channel_url = self.url(),
            channel_feed = self.rss_url()
        )
    }
}

impl Channel {
    pub fn url(&self) -> String {
        format!("https://www.youtube.com/{}", self.handle)
    }

    pub fn rss_url(&self) -> String {
        format!(
            "{}/feeds/videos.xml?channel_id={}",
            RSS_HOST, self.channel_id
        )
    }

    pub async fn validate_id(channel_id: &str, host: Option<&str>) -> Result<bool> {
        let host = host.unwrap_or(RSS_HOST);
        let client = Client::new();
        let res = client
            .get(format!(
                "{}/feeds/videos.xml?channel_id={}",
                host, channel_id,
            ))
            .send()
            .await?;

        Ok(res.status() == 200)
    }

    /// Resolve a channel's `(channel_id, channel_name)` from a handle/URL.
    ///
    /// Tries the proxied YouTube API first. If that host is down, throttled or
    /// returns a non-JSON body, falls back to reading the channel page directly
    /// from YouTube so `follow` keeps working.
    pub async fn get_data(handle: &str, host: Option<&str>) -> Result<(String, String)> {
        match Self::get_data_via_api(handle, host).await {
            Ok(data) => Ok(data),
            Err(api_err) => match Self::get_data_from_youtube(handle, None).await {
                Ok(data) => Ok(data),
                Err(yt_err) => Err(eyre!(
                    "Could not resolve channel '{handle}'.\n  API ({}): {api_err}\n  YouTube: {yt_err}\n\n\
                     Try again later, or supply the data manually:\n\
                     https://github.com/pawurb/yt-sub-rs#manually-finding-an-rss-channel_id",
                    host.unwrap_or(API_HOST)
                )),
            },
        }
    }

    async fn get_data_via_api(handle: &str, host: Option<&str>) -> Result<(String, String)> {
        let host = host.unwrap_or(API_HOST);
        let client = Client::new();

        let res = client
            .get(format!("{}/channel_data/{}", host, handle))
            .send()
            .await?;
        let status = res.status();

        if status == StatusCode::NOT_FOUND {
            eyre::bail!("Channel with handle '{handle}' not found!")
        }

        if status == StatusCode::SERVICE_UNAVAILABLE {
            eyre::bail!("YouTube API calls are currently throttled (HTTP 503)")
        }

        if !status.is_success() {
            let body = res.text().await.unwrap_or_default();
            eyre::bail!("API returned HTTP {status}: {}", body.trim())
        }

        let res_json: Value = res
            .json()
            .await
            .map_err(|e| eyre!("API returned invalid JSON (HTTP {status}): {e}"))?;

        let channel_id = res_json["channel_id"]
            .as_str()
            .ok_or_else(|| eyre!("API response missing 'channel_id'"))?;
        let channel_name = res_json["channel_name"]
            .as_str()
            .ok_or_else(|| eyre!("API response missing 'channel_name'"))?;

        Ok((channel_id.to_string(), channel_name.to_string()))
    }

    async fn get_data_from_youtube(handle: &str, host: Option<&str>) -> Result<(String, String)> {
        let host = host.unwrap_or(YOUTUBE_HOST);
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| eyre!("Failed to build HTTP client: {e}"))?;

        let res = client
            .get(format!("{host}/{handle}"))
            .header("Accept-Language", "en-US,en;q=0.9")
            .header("Cookie", CONSENT_COOKIE)
            .send()
            .await?;
        let status = res.status();

        if status == StatusCode::NOT_FOUND {
            eyre::bail!("Channel with handle '{handle}' not found!")
        }

        if !status.is_success() {
            eyre::bail!("YouTube returned HTTP {status}")
        }

        let html = res.text().await?;

        let channel_id = Self::extract_between(&html, "\"externalId\":\"", "\"")
            .or_else(|| Self::extract_between(&html, "\"channelId\":\"", "\""))
            .filter(|id| id.starts_with("UC"))
            .ok_or_else(|| eyre!("Could not find a channel_id in the YouTube page"))?;

        let channel_name = Self::extract_between(&html, "<meta property=\"og:title\" content=\"", "\"")
            .or_else(|| Self::extract_between(&html, "\"channelMetadataRenderer\":{\"title\":\"", "\""))
            .unwrap_or_else(|| handle.to_string());

        Ok((channel_id.to_string(), channel_name))
    }

    /// Return the substring between the first occurrence of `start` and the next `end`.
    fn extract_between(haystack: &str, start: &str, end: &str) -> Option<String> {
        let rest = haystack.split_once(start)?.1;
        Some(rest.split_once(end)?.0.to_string())
    }

    pub async fn get_fresh_videos(&self, last_run_at: DateTime<Utc>) -> Result<Vec<Video>> {
        let rss = self.get_rss_data().await?;
        let videos = Video::parse_rss(rss, Some(self.handle.clone()))?;

        let videos: Vec<Video> = videos
            .into_iter()
            .filter(|video| video.published_at > last_run_at)
            .collect();

        Ok(videos)
    }

    async fn get_rss_data(&self) -> Result<String> {
        let client = Client::new();
        let res = client.get(self.rss_url()).send().await?;
        Ok(res.text().await?)
    }
}

#[cfg(test)]
mod tests {
    use mockito::Server;

    use super::*;
    #[tokio::test]
    async fn test_validate_channel() -> Result<()> {
        let mut server = Server::new_async().await;
        let host = server.host_with_port();
        let host = format!("http://{}", host);
        let m1 = server
            .mock(
                "GET",
                "/feeds/videos.xml?channel_id=UC_iD0xppBwwsrM9DegC5cQQ",
            )
            .with_status(200)
            .create_async()
            .await;
        let m2 = server
            .mock("GET", "/feeds/videos.xml?channel_id=UC_invalid")
            .with_status(404)
            .create_async()
            .await;

        let correct_res = Channel::validate_id("UC_iD0xppBwwsrM9DegC5cQQ", Some(&host)).await?;
        assert!(correct_res);

        let incorrect_res = Channel::validate_id("UC_invalid", Some(&host)).await?;
        assert!(!incorrect_res);

        m1.assert_async().await;
        m2.assert_async().await;

        Ok(())
    }

    #[tokio::test]
    async fn test_get_channel_data() -> Result<()> {
        let mut server = Server::new_async().await;
        let host = server.host_with_port();
        let host = format!("http://{}", host);
        let m = server
            .mock("GET", "/channel_data/@Test_handle")
            .with_body(
                r#"{
            "channel_id": "UC_iD0xppBwwsrM9DegC5cQQ",
            "channel_name": "Test Channel"
        }"#,
            )
            .create_async()
            .await;

        let (channel_id, channel_name) = Channel::get_data("@Test_handle", Some(&host)).await?;
        assert_eq!(channel_id, "UC_iD0xppBwwsrM9DegC5cQQ");
        assert_eq!(channel_name, "Test Channel");

        m.assert_async().await;

        Ok(())
    }

    #[tokio::test]
    async fn test_api_502_gives_clear_error_not_decode_failure() -> Result<()> {
        let mut server = Server::new_async().await;
        let host = server.host_with_port();
        let host = format!("http://{}", host);

        let m = server
            .mock("GET", "/channel_data/@Xaryu")
            .with_status(502)
            .with_body("error code: 502")
            .create_async()
            .await;

        let err = Channel::get_data_via_api("@Xaryu", Some(&host))
            .await
            .unwrap_err()
            .to_string();

        assert!(err.contains("HTTP 502"), "unexpected error: {err}");
        assert!(
            !err.contains("error decoding response body"),
            "should not surface a raw decode error: {err}"
        );

        m.assert_async().await;
        Ok(())
    }

    #[tokio::test]
    async fn test_get_data_from_youtube_scrapes_page() -> Result<()> {
        let mut server = Server::new_async().await;
        let host = server.host_with_port();
        let host = format!("http://{}", host);

        let html = r#"<html><head>
            <meta property="og:title" content="Xaryu">
            </head><body>"externalId":"UCAbaiKvP8kZfY706loT4ivg"</body></html>"#;

        let m = server
            .mock("GET", "/@Xaryu")
            .with_status(200)
            .with_body(html)
            .create_async()
            .await;

        let (channel_id, channel_name) =
            Channel::get_data_from_youtube("@Xaryu", Some(&host)).await?;
        assert_eq!(channel_id, "UCAbaiKvP8kZfY706loT4ivg");
        assert_eq!(channel_name, "Xaryu");

        m.assert_async().await;
        Ok(())
    }
}
