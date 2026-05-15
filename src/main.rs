use clap::Parser;

use spotify_dl::download::{DownloadOptions, Downloader};
use spotify_dl::encoder::Format;
use spotify_dl::log;
use spotify_dl::session::create_session;
use spotify_dl::track::get_tracks;

#[derive(Debug, Parser)]
#[command(
    name = "spotify-dl",
    about = "A commandline utility to download music directly from Spotify"
)]
struct Opt {
    /// A list of Spotify URIs or URLs (songs, podcasts, playlists or albums)
    #[arg(required = true)]
    tracks: Vec<String>,

    /// The directory where the songs will be downloaded
    #[arg(short = 'd', long = "destination")]
    destination: Option<String>,

    /// Number of parallel downloads. Default is 5.
    #[arg(short = 't', long = "parallel", default_value = "5")]
    parallel: usize,

    /// The format to download the tracks in. Default is flac.
    #[arg(short = 'f', long = "format", default_value = "flac")]
    format: Format,

    /// Force download even if the file already exists
    #[arg(short = 'F', long = "force")]
    force: bool,

    /// Delay in seconds between downloading each track. Default is 60.
    #[arg(short = 'r', long = "rate-limit", default_value = "60")]
    rate_limit: u64,
}

pub fn create_destination_if_required(destination: Option<String>) -> anyhow::Result<()> {
    if let Some(destination) = destination {
        if !std::path::Path::new(&destination).exists() {
            tracing::info!("Creating destination directory: {}", destination);
            std::fs::create_dir_all(destination)?;
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    log::configure_logger()?;

    let opt = Opt::parse();
    create_destination_if_required(opt.destination.clone())?;

    if opt.tracks.is_empty() {
        eprintln!("No tracks provided");
        std::process::exit(1);
    }

    let session = create_session().await?;

    let track = get_tracks(opt.tracks, &session).await?;
    println!("  {} tracks to process\n", track.len());

    let downloader = Downloader::new(session);
    downloader
        .download_tracks(
            track,
            &DownloadOptions::new(opt.destination, opt.parallel, opt.format, opt.force, opt.rate_limit),
        )
        .await
}
