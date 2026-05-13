use std::fmt::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use indicatif::MultiProgress;
use indicatif::ProgressBar;
use indicatif::ProgressState;
use indicatif::ProgressStyle;
use librespot::core::session::Session;

use crate::encoder;
use crate::encoder::Format;
use crate::encoder::Samples;
use crate::stream::Stream;
use crate::stream::StreamEvent;
use crate::stream::StreamEventChannel;
use crate::track::Track;
use crate::track::TrackMetadata;

pub struct Downloader {
    session: Session,
    progress_bar: MultiProgress,
}

#[derive(Debug, Clone)]
pub struct DownloadOptions {
    pub destination: PathBuf,
    pub parallel: usize,
    pub format: Format,
    pub force: bool,
    pub rate_limit_secs: u64,
}

impl DownloadOptions {
    pub fn new(destination: Option<String>, parallel: usize, format: Format, force: bool, rate_limit_secs: u64) -> Self {
        let destination =
            destination.map_or_else(|| std::env::current_dir().unwrap(), PathBuf::from);
        DownloadOptions {
            destination,
            parallel,
            format,
            force,
            rate_limit_secs,
        }
    }
}

impl Downloader {
    pub fn new(session: Session) -> Self {
        Downloader {
            session,
            progress_bar: MultiProgress::new(),
        }
    }

    pub async fn download_tracks(
        self,
        tracks: Vec<Track>,
        options: &DownloadOptions,
    ) -> Result<()> {
        let total = tracks.len();
        let rate_limit = Duration::from_secs(options.rate_limit_secs);

        if options.rate_limit_secs > 0 {
            println!("  ⏱ Rate limit: {}s delay between tracks\n", options.rate_limit_secs);
        }

        let mut last_was_download = false;

        for (index, track) in tracks.into_iter().enumerate() {
            // Rate-limit: sleep only after an actual download, not after skips/renames
            if last_was_download && options.rate_limit_secs > 0 {
                tracing::info!("Rate limiting: waiting {}s before next track", options.rate_limit_secs);
                tokio::time::sleep(rate_limit).await;
            }

            last_was_download = self.download_track(track, options, index + 1, total).await?;
        }

        Ok(())
    }

    #[tracing::instrument(name = "download_track", skip(self))]
    /// Returns Ok(true) if a download was performed, Ok(false) if skipped/renamed.
    async fn download_track(&self, track: Track, options: &DownloadOptions, position: usize, total: usize) -> Result<bool> {
        let counter = format!("[{}/{}]", position, total);
        let metadata = track.metadata(&self.session).await?;
        tracing::info!("Downloading track: {:?}", metadata.track_name);

        let path = options
            .destination
            .join(metadata.to_string())
            .with_extension(options.format.extension())
            .to_str()
            .ok_or(anyhow::anyhow!("Could not set the output path"))?
            .to_string();

        if !options.force && PathBuf::from(&path).exists() {
            tracing::info!(
                "Skipping {}, file already exists. Use --force to force re-downloading the track",
                &metadata.track_name
            );
            println!("  {} Skipped {} (already exists)", counter, metadata.to_string());
            return Ok(false);
        }

        // Smart rename: check for existing files under old naming conventions
        if !options.force && metadata.position.is_some() {
            let ext = options.format.extension();
            let bare_name = metadata.to_string_without_position();
            let expected_path = PathBuf::from(&path);

            // Case 1: File exists without track number prefix (old format: "Artist - Song.ext")
            let bare_path = options.destination.join(&bare_name).with_extension(ext);
            if bare_path.exists() {
                tracing::info!("Renaming {} -> {}", bare_path.display(), expected_path.display());
                std::fs::rename(&bare_path, &expected_path)?;
                // Update the track number tag in the renamed file
                let mut tags = metadata.tags().await?;
                tags.track_number = Some(position as u16);
                tags.disc_number = None;
                crate::encoder::tags::store_tags(path.clone(), &tags, options.format).await?;
                println!("  {} Renamed {} → {}", counter, bare_path.file_name().unwrap().to_string_lossy(), expected_path.file_name().unwrap().to_string_lossy());
                return Ok(false);
            }

            // Case 2: File exists with a different/stale track number (e.g. playlist reorder)
            let stale_pattern = format!(" - {}", bare_name);
            if let Ok(entries) = std::fs::read_dir(&options.destination) {
                for entry in entries.filter_map(|e| e.ok()) {
                    let file_name = entry.file_name().to_string_lossy().to_string();
                    if let Some(stem) = file_name.strip_suffix(&format!(".{}", ext)) {
                        // Match pattern: "NN - Artist - Song" where NN differs from current position
                        if stem.ends_with(&stale_pattern)
                            && stem.len() > stale_pattern.len()
                        {
                            let prefix = &stem[..stem.len() - stale_pattern.len()];
                            if prefix.len() >= 2 && prefix.chars().all(|c| c.is_ascii_digit()) {
                                let old_path = entry.path();
                                if old_path != expected_path {
                                    tracing::info!("Renaming (stale number) {} -> {}", old_path.display(), expected_path.display());
                                    std::fs::rename(&old_path, &expected_path)?;
                                    let mut tags = metadata.tags().await?;
                                    tags.track_number = Some(position as u16);
                                    tags.disc_number = None;
                                    crate::encoder::tags::store_tags(path.clone(), &tags, options.format).await?;
                                    println!("  {} Renamed {} → {}", counter, old_path.file_name().unwrap().to_string_lossy(), expected_path.file_name().unwrap().to_string_lossy());
                                    return Ok(false);
                                }
                            }
                        }
                    }
                }
            }
        }

        let pb = self.add_progress_bar(&metadata, &counter);

        let stream = Stream::new(self.session.clone());
        let channel = match stream.stream(Arc::new(track)).await {
            Ok(channel) => channel,
            Err(e) => {
                self.fail_with_error(&pb, &metadata.to_string(), e.to_string(), &counter);
                return Ok(false);
            }
        };

        let samples = match self.buffer_track(channel, &pb, &metadata, &counter).await {
            Ok(samples) => samples,
            Err(e) => {
                self.fail_with_error(&pb, &metadata.to_string(), e.to_string(), &counter);
                return Ok(false);
            }
        };

        tracing::info!("Encoding track: {}", metadata.to_string());
        pb.set_message(format!("{} Encoding {}", counter, metadata.to_string()));

        let encoder = crate::encoder::get_encoder(options.format);
        let stream = encoder.encode(samples).await?;

        pb.set_message(format!("{} Writing {}", counter, metadata.to_string()));
        tracing::info!(
            "Writing track: {:?} to file: {}",
            metadata.to_string(),
            &path
        );
        stream.write_to_file(&path).await?;

        let mut tags = metadata.tags().await?;
        // Override track number with playlist position
        tags.track_number = Some(position as u16);
        tags.disc_number = None; // disc number not meaningful for playlists
        encoder::tags::store_tags(path, &tags, options.format).await?;

        pb.finish_with_message(format!("{} Downloaded {}", counter, metadata.to_string()));
        Ok(true)
    }

    fn add_progress_bar(&self, track: &TrackMetadata, counter: &str) -> ProgressBar {
        let pb = self
            .progress_bar
            .add(ProgressBar::new(track.approx_size() as u64));
        pb.enable_steady_tick(Duration::from_millis(100));
        pb.set_style(ProgressStyle::with_template("{spinner:.green} {msg} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({eta})")
            // Infallible
            .unwrap()
            .with_key("eta", |state: &ProgressState, w: &mut dyn Write| write!(w, "{:.1}s", state.eta().as_secs_f64()).unwrap())
            .progress_chars("#>-"));
        pb.set_message(format!("{} {}", counter, track.to_string()));
        pb
    }

    async fn buffer_track(
        &self,
        mut rx: StreamEventChannel,
        pb: &ProgressBar,
        metadata: &TrackMetadata,
        counter: &str,
    ) -> Result<Samples> {
        let mut samples = Vec::<i32>::new();
        while let Some(event) = rx.recv().await {
            match event {
                StreamEvent::Write {
                    bytes,
                    total,
                    mut content,
                } => {
                    tracing::trace!("Written {} bytes out of {}", bytes, total);
                    pb.set_position(bytes as u64);
                    samples.append(&mut content);
                }
                StreamEvent::Finished => {
                    tracing::info!("Finished downloading track");
                    break;
                }
                StreamEvent::Error(stream_error) => {
                    tracing::error!("Error while streaming track: {:?}", stream_error);
                    return Err(anyhow::anyhow!("Streaming error: {:?}", stream_error));
                }
                StreamEvent::Retry {
                    attempt,
                    max_attempts,
                } => {
                    tracing::warn!(
                        "Retrying download, attempt {} of {}: {}",
                        attempt,
                        max_attempts,
                        metadata.to_string()
                    );
                    pb.set_message(format!(
                        "{} Retrying ({}/{}) {}",
                        counter,
                        attempt,
                        max_attempts,
                        metadata.to_string()
                    ));
                }
            }
        }
        Ok(Samples {
            samples,
            ..Default::default()
        })
    }

    fn fail_with_error<S>(&self, pb: &ProgressBar, name: &str, e: S, counter: &str)
    where
        S: Into<String>,
    {
        tracing::error!("Failed to download {}: {}", name, e.into());
        pb.finish_with_message(console::style(format!("{} Failed! {}", counter, name)).red().to_string());
    }
}
