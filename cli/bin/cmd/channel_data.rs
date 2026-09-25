use clap::Parser;
use eyre::Result;
use yt_sub_core::channel::Channel;

#[derive(Debug, Parser)]
pub struct ChannelDataArgs {
    /// Channel handle (@handle) or YouTube URL
    target: String,

    #[arg(long, hide = true)]
    handle: Option<String>,
}

impl ChannelDataArgs {
    pub async fn run(self) -> Result<()> {
        let Self { target, handle } = self;

        let handle = handle.unwrap_or(target);
        let handle = if handle.starts_with('@') {
            handle
        } else if let Some(rest) = handle
            .strip_prefix("https://www.youtube.com/")
            .or_else(|| handle.strip_prefix("https://youtube.com/"))
        {
            let path = rest.split('?').next().unwrap_or(rest).trim_end_matches('/');
            if path.starts_with('@') {
                path.to_string()
            } else if let Some(name) = path.strip_prefix("c/").or_else(|| path.strip_prefix("channel/")) {
                format!("@{name}")
            } else {
                format!("@{path}")
            }
        } else {
            format!("@{handle}")
        };

        let (channel_id, channel_name) = Channel::get_data(&handle, None).await?;

        let channel = Channel {
            handle: handle.clone(),
            description: channel_name.clone(),
            channel_id: channel_id.clone(),
        };

        println!(
            "{channel}

Run: 

ytsub follow {handle}

to subscribe to this channel.",
            handle = channel.handle,
        );
        Ok(())
    }
}
