use chrono::Utc;
use clap::Parser;
use eyre::Result;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use yt_sub::user_settings_cli::UserSettingsCLI;
use yt_sub_core::{logger::Logger, notifier::Notifier, UserSettings};

use crate::CONFIG_DESC;

#[derive(Debug, Parser)]
pub struct RunArgs {
    #[arg(long, help = CONFIG_DESC)]
    config: Option<PathBuf>,

    #[arg(long, help = "Produce cron-style std logs")]
    cron: bool,

    #[arg(long, help = "Fresh videos hours offset")]
    hours_offset: Option<u16>,

    #[arg(long, conflicts_with = "output_append", help = "Write output to file (overwrites existing file)")]
    output: Option<PathBuf>,

    #[arg(long, conflicts_with = "output", help = "Append output to file")]
    output_append: Option<PathBuf>,
}

impl RunArgs {
    pub async fn run(self) -> Result<()> {
        let Self {
            config,
            cron,
            hours_offset,
            output,
            output_append,
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

        // Write output to file if requested
        if let Some(output_path) = output {
            let lines = new_videos
                .iter()
                .map(|v| v.notification_text(&Notifier::Log()))
                .collect::<Vec<String>>();
            write_output_to_file(&output_path, &lines, false)?;
        }

        if let Some(output_path) = output_append {
            let lines = new_videos
                .iter()
                .map(|v| v.notification_text(&Notifier::Log()))
                .collect::<Vec<String>>();
            write_output_to_file(&output_path, &lines, true)?;
        }

        settings.touch_last_run_at()?;

        Ok(())
    }
}

fn write_output_to_file(
    path: &std::path::Path,
    lines: &[String],
    append: bool,
) -> Result<()> {
    if lines.is_empty() {
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }

    let mut file = if append {
        OpenOptions::new().append(true).create(true).open(path)?
    } else {
        OpenOptions::new().write(true).create(true).truncate(true).open(path)?
    };

    file.write_all(lines.join("\n").as_bytes())?;
    file.write_all(b"\n")?;

    Ok(())
}
