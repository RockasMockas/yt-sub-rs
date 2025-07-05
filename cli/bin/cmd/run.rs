use chrono::Utc;
use clap::Parser;
use eyre::Result;
use std::path::PathBuf;
use yt_sub::user_settings_cli::UserSettingsCLI;
use yt_sub_core::{logger::Logger, UserSettings};

use crate::CONFIG_DESC;

#[derive(Debug, Parser)]
pub struct RunArgs {
    #[arg(long, help = CONFIG_DESC)]
    config: Option<PathBuf>,

    #[arg(long, help = "Produce cron-style std logs")]
    cron: bool,

    #[arg(long, help = "Fresh videos hours offset")]
    hours_offset: Option<u16>,
}

impl RunArgs {
    pub async fn run(self) -> Result<()> {
        let Self {
            config,
            cron,
            hours_offset,
        } = self;

        let logger = Logger::new(cron);

        let settings = UserSettings::read(config.as_ref())?;

        let last_run_at = if let Some(hours_offset) = hours_offset {
            Utc::now() - chrono::Duration::hours(hours_offset as i64)
        } else {
            settings.last_run_at()
        };

        let mut new_videos = vec![];

        for channel in &settings.channels {
            match channel.get_fresh_videos(last_run_at).await {
                Ok(videos) => {
                    new_videos.extend(videos);
                }
                Err(e) => {
                    logger.error(&format!("Error: {e}"));
                }
            }
        }

        if new_videos.is_empty() {
            logger.info("No new videos found.");
            return Ok(());
        }

        // Sort videos by publication date (newest first)
        new_videos.sort_by(|a, b| b.published_at.cmp(&a.published_at));

        for notifier in &settings.notifiers {
            let notifications = new_videos
                .iter()
                .map(|video| video.notification_text(notifier))
                .collect::<Vec<String>>();

            match notifier.notify(notifications, cron).await {
                Ok(_) => {}
                Err(e) => {
                    logger.error(&format!("Error: {e}"));
                }
            }
        }

        settings.touch_last_run_at()?;

        Ok(())
    }
}
