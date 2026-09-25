use std::path::PathBuf;

use clap::Parser;
use eyre::Result;
use yt_sub::user_settings_cli::UserSettingsCLI;
use yt_sub_core::{channel::Channel, UserSettings};

use crate::CONFIG_DESC;

#[derive(Debug, Parser)]
pub struct FollowArgs {
    #[arg(long, help = CONFIG_DESC)]
    config: Option<PathBuf>,

    /// Channel handle (@handle) or YouTube URL
    #[arg(required_unless_present_any = ["channel_id", "handle"])]
    target: Option<String>,

    #[arg(long)]
    handle: Option<String>,
    #[arg(long)]
    channel_id: Option<String>,
    #[arg(long)]
    desc: Option<String>,
}

/// Extract a handle from a YouTube URL or pass through a bare @handle.
fn parse_handle(input: &str) -> String {
    let input = input.trim();

    // Already a bare handle
    if input.starts_with('@') {
        return input.to_string();
    }

    // Try to extract handle from URL
    // Formats: https://www.youtube.com/@handle, https://youtube.com/@handle,
    //          https://www.youtube.com/channel/UC..., https://www.youtube.com/c/name
    if let Some(rest) = input
        .strip_prefix("https://www.youtube.com/")
        .or_else(|| input.strip_prefix("https://youtube.com/"))
        .or_else(|| input.strip_prefix("http://www.youtube.com/"))
        .or_else(|| input.strip_prefix("http://youtube.com/"))
    {
        // Strip trailing slash and query params
        let path = rest.split('?').next().unwrap_or(rest);
        let path = path.trim_end_matches('/');

        if path.starts_with('@') {
            return path.to_string();
        }
        if let Some(channel_id) = path.strip_prefix("channel/") {
            return format!("channel/{channel_id}");
        }
        if let Some(name) = path.strip_prefix("c/") {
            return format!("@{name}");
        }
        // Fallback: treat last segment as handle
        if let Some(last) = path.rsplit('/').next() {
            if !last.is_empty() {
                return format!("@{last}");
            }
        }
    }

    // Bare name without @ — add it
    format!("@{input}")
}

impl FollowArgs {
    pub async fn run(self) -> Result<()> {
        let Self {
            channel_id,
            desc,
            handle,
            target,
            config,
        } = self;

        let handle = if let Some(t) = target {
            parse_handle(&t)
        } else if let Some(h) = handle {
            parse_handle(&h)
        } else {
            eyre::bail!("Provide a channel handle (@handle), YouTube URL, or --channel-id + --desc");
        };

        if (channel_id.is_none() && desc.is_some()) || (channel_id.is_some() && desc.is_none()) {
            eyre::bail!("You must provide only a handle/URL or both --channel-id and --desc");
        }

        let (channel_id, desc) = if channel_id.is_none() && desc.is_none() {
            Channel::get_data(&handle, None).await?
        } else {
            (channel_id.unwrap(), desc.unwrap())
        };

        let settings = UserSettings::read(config.as_ref())?;
        let already_following_id = settings.get_channel_by_id(&channel_id);
        let already_following_handle = settings.get_channel_by_handle(&handle);

        if already_following_id.is_some() || already_following_handle.is_some() {
            let following =
                already_following_id.unwrap_or_else(|| already_following_handle.unwrap());

            eyre::bail!("You are already following this channel! \n\n{following}");
        }

        let mut channels = settings.channels;

        let channel = Channel {
            handle,
            description: desc.clone(),
            channel_id,
        };

        channels.push(channel.clone());
        let settings = UserSettings {
            channels,
            ..settings
        };

        settings.save(config.as_ref())?;

        println!(
            "You are now following:

{channel}"
        );

        if settings.api_key.is_some() {
            match settings.sync_account(None).await {
                Ok(_) => {
                    println!("Remote account data was updated.");
                }
                Err(e) => {
                    eprintln!("Error: {}", e)
                }
            }
        }

        Ok(())
    }
}
