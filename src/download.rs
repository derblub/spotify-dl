use std::collections::HashMap;
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

/// Result of checking whether a track needs downloading.
enum TrackAction {
    /// Track needs to be downloaded from Spotify.
    Download {
        path: String,
    },
    /// Track was skipped (file already exists).
    Skipped {
        display_name: String,
    },
    /// Track was renamed from an old naming convention.
    Renamed {
        from: String,
        to: String,
        display_name: String,
    },
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

        // Pre-scan destination directory for O(n) rename detection.
        // Maps bare name (without track-number prefix) to full path.
        let mut existing_files: HashMap<String, PathBuf> = HashMap::new();
        if let Ok(entries) = std::fs::read_dir(&options.destination) {
            let ext = options.format.extension();
            for entry in entries.filter_map(|e| e.ok()) {
                let file_name = entry.file_name().to_string_lossy().to_string();
                if let Some(stem) = file_name.strip_suffix(&format!(".{}", ext)) {
                    // Extract bare name: strip leading "NN - " prefix if present
                    let bare = strip_track_number_prefix(stem);
                    existing_files.insert(bare.to_string(), entry.path());
                }
            }
        }

        let mut last_download_at: Option<tokio::time::Instant> = None;

        for (index, track) in tracks.into_iter().enumerate() {
            let position = index + 1;
            let counter = format!("[{}/{}]", position, total);
            let metadata = track.metadata(&self.session).await?;
            let display_name = metadata.to_string();

            // Phase 1: Check what action is needed for this track
            let action = self.check_track_status(
                &metadata,
                &display_name,
                options,
                &existing_files,
            );

            match action {
                TrackAction::Download { ref path } => {
                    // Rate-limit: if a previous download happened, wait for remaining time
                    if let Some(last_at) = last_download_at {
                        let elapsed = last_at.elapsed();
                        if elapsed < rate_limit {
                            let remaining = rate_limit - elapsed;
                            self.show_rate_limit_countdown(remaining).await;
                        }
                    }

                    // Phase 2: Perform the actual download
                    let downloaded = self.perform_download(
                        track,
                        &metadata,
                        &display_name,
                        path,
                        options,
                        position,
                        &counter,
                    ).await?;

                    if downloaded {
                        last_download_at = Some(tokio::time::Instant::now());
                    }
                }
                TrackAction::Skipped { ref display_name } => {
                    println!("  {} Skipped {} (already exists)", counter, display_name);
                    // Don't reset timer — let it keep counting through skips
                }
                TrackAction::Renamed { ref from, ref to, ref display_name } => {
                    println!("  {} Renamed {} → {}", counter, from, to);
                    // Don't reset timer — let it keep counting through renames

                    // Update track number tag in the renamed file
                    let renamed_path = options.destination.join(display_name)
                        .with_extension(options.format.extension());
                    if let Ok(mut tags) = metadata.tags().await {
                        tags.track_number = Some(position as u16);
                        tags.disc_number = None;
                        let _ = encoder::tags::store_tags(
                            renamed_path.to_string_lossy().to_string(),
                            &tags,
                            options.format,
                        ).await;
                    }

                    // Update the pre-scan index: remove old entry, add new one
                    let bare = metadata.to_string_without_position();
                    existing_files.remove(&bare);
                    existing_files.insert(bare, renamed_path);
                }
            }
        }

        Ok(())
    }

    /// Check whether a track needs downloading, can be skipped, or should be renamed.
    fn check_track_status(
        &self,
        metadata: &TrackMetadata,
        display_name: &str,
        options: &DownloadOptions,
        existing_files: &HashMap<String, PathBuf>,
    ) -> TrackAction {
        let path = options
            .destination
            .join(display_name)
            .with_extension(options.format.extension())
            .to_str()
            .unwrap_or_default()
            .to_string();

        // Already exists with correct name
        if !options.force && PathBuf::from(&path).exists() {
            tracing::info!(
                "Skipping {}, file already exists. Use --force to force re-downloading the track",
                &metadata.track_name
            );
            return TrackAction::Skipped {
                display_name: display_name.to_string(),
            };
        }

        // Smart rename: check for existing files under old naming conventions
        if !options.force && metadata.position.is_some() {
            let bare_name = metadata.to_string_without_position();
            let expected_path = PathBuf::from(&path);

            // Use pre-scanned index for O(1) lookup instead of O(n) directory scan
            if let Some(old_path) = existing_files.get(&bare_name) {
                if old_path != &expected_path && old_path.exists() {
                    let from_name = old_path.file_name().unwrap().to_string_lossy().to_string();
                    let to_name = expected_path.file_name().unwrap().to_string_lossy().to_string();

                    tracing::info!("Renaming {} -> {}", old_path.display(), expected_path.display());
                    if let Err(e) = std::fs::rename(old_path, &expected_path) {
                        tracing::error!("Failed to rename: {}", e);
                    } else {
                        return TrackAction::Renamed {
                            from: from_name,
                            to: to_name,
                            display_name: display_name.to_string(),
                        };
                    }
                }
            }
        }

        TrackAction::Download {
            path,
        }
    }

    /// Perform the actual download, encode, and write for a track.
    /// Returns Ok(true) if the download completed successfully.
    #[tracing::instrument(name = "perform_download", skip(self, track, metadata))]
    async fn perform_download(
        &self,
        track: Track,
        metadata: &TrackMetadata,
        display_name: &str,
        path: &str,
        options: &DownloadOptions,
        position: usize,
        counter: &str,
    ) -> Result<bool> {
        tracing::info!("Downloading track: {:?}", metadata.track_name);

        let pb = self.add_progress_bar(metadata, counter);

        let stream = Stream::new(self.session.clone());
        let channel = match stream.stream(Arc::new(track)).await {
            Ok(channel) => channel,
            Err(e) => {
                self.fail_with_error(&pb, display_name, e.to_string(), counter);
                return Ok(false);
            }
        };

        let samples = match self.buffer_track(channel, &pb, metadata, counter).await {
            Ok(samples) => samples,
            Err(e) => {
                self.fail_with_error(&pb, display_name, e.to_string(), counter);
                return Ok(false);
            }
        };

        tracing::info!("Encoding track: {}", display_name);
        pb.set_message(format!("{} Encoding {}", counter, display_name));

        let encoder = crate::encoder::get_encoder(options.format);
        let stream = encoder.encode(samples).await?;

        pb.set_message(format!("{} Writing {}", counter, display_name));
        tracing::info!(
            "Writing track: {:?} to file: {}",
            display_name,
            path
        );
        stream.write_to_file(path).await?;

        let mut tags = metadata.tags().await?;
        // Override track number with playlist position
        tags.track_number = Some(position as u16);
        tags.disc_number = None; // disc number not meaningful for playlists
        encoder::tags::store_tags(path.to_string(), &tags, options.format).await?;

        pb.finish_with_message(format!("{} Downloaded {}", counter, display_name));
        Ok(true)
    }

    /// Wait for the rate limit to expire.
    async fn show_rate_limit_countdown(&self, remaining: Duration) {
        if remaining.is_zero() {
            return;
        }
        tokio::time::sleep(remaining).await;
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
        let display_name = metadata.to_string();
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
                        display_name
                    );
                    pb.set_message(format!(
                        "{} Retrying ({}/{}) {}",
                        counter,
                        attempt,
                        max_attempts,
                        display_name
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

/// Strip a leading track-number prefix like "01 - " from a filename stem.
/// Returns the bare name portion.
fn strip_track_number_prefix(stem: &str) -> &str {
    // Match pattern: two or more digits followed by " - "
    if stem.len() >= 5 {
        if let Some(pos) = stem.find(" - ") {
            let prefix = &stem[..pos];
            if prefix.len() >= 2 && prefix.chars().all(|c| c.is_ascii_digit()) {
                return &stem[pos + 3..];
            }
        }
    }
    stem
}
