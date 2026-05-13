# 🎵 spotify-dl

A command-line utility to download songs, playlists, and albums directly from Spotify — with **batch library sync**, **folder hierarchy preservation**, and an **interactive TUI** for single-playlist downloads.

> [!IMPORTANT]
> A Spotify Premium account is required.

> [!CAUTION]
> Usage of this software may infringe Spotify's terms of service or your local legislation. Use it under your own risk.

## 🚀 Features

- **Full library sync** — downloads every playlist in your Spotify library, preserving folder structure.
- **Interactive single-download mode** — fuzzy-search a playlist and download it with a TUI form (`fzf` integration).
- **Folder hierarchy** — uses [`spotifyfolders`](https://github.com/mikez/spotifyfolders) to mirror your Spotify folder tree on disk.
- **Track-number prefixed filenames** — files are named `01 - Artist - Title` for proper album ordering.
- **Smart skip & rename** — existing files are detected and renamed in-place when only the track number prefix changed, avoiding re-downloads.
- **Rate limiting** — configurable delay between track downloads (default 60 s) to stay under Spotify's radar.
- **Sequential downloads** — uses `-t 1` by default to prevent session concurrency errors.
- **Playlist name resolution** — multi-layer strategy: local cache → Spotify Web API → web scraping fallback.
- **Metadata tagging** — embeds artist, album, track number, and disc number into downloaded files.
- **Formats** — MP3 (320 kbps), FLAC, or OGG.
- **Credentials in `.env`** — secrets are kept out of version control.

## 📦 Prerequisites

| Dependency | Purpose |
|---|---|
| [Rust toolchain](https://rustup.rs/) | Building `spotify-dl` |
| `jq` | JSON parsing in shell scripts |
| `curl` | Spotify Web API calls |
| `python3` | Walking the folder hierarchy JSON |
| [`spotifyfolders`](https://github.com/mikez/spotifyfolders) | Exporting your Spotify folder tree |
| [`fzf`](https://github.com/junegunn/fzf) | *(optional)* Fuzzy playlist picker in `download_single.sh` |
| A web server with PHP | *(optional)* `spotify_code.php` OAuth callback helper |

## ⚙️ Setup

### 1. Build the binary

```bash
cargo build --release
```

The binary is compiled to `./target/release/spotify-dl`.

### 2. Create a Spotify App

1. Go to the [Spotify Developer Dashboard](https://developer.spotify.com/dashboard).
2. Create a new app.
3. Add your redirect URI (e.g. `https://example.com/callback`).
4. Note down the **Client ID** and **Client Secret**.

### 3. Configure `.env`

```bash
cp .env.example .env
```

Edit `.env` with your credentials:

```env
SPOTIFY_CLIENT_ID=your_client_id_here
SPOTIFY_CLIENT_SECRET=your_client_secret_here
SPOTIFY_REDIRECT_URI=https://example.com/callback

OUTPUT_PATH=/path/to/music/output
DOWNLOAD_FORMAT=mp3
```

> [!NOTE]
> The `.env` file is git-ignored. Never commit your credentials.

## 🧭 Usage

### Full library sync — `start.sh`

Syncs **every** playlist in your Spotify library, preserving the folder hierarchy from the Spotify desktop client.

```bash
./start.sh
```

**What it does:**

1. Runs `spotifyfolders` to capture your folder tree → `spotifyfolders.json`.
2. Authenticates via OAuth (you'll paste an authorization code).
3. Batch-fetches all playlist names from the Spotify API.
4. Resolves any remaining names (API → web scrape → fallback ID).
5. Downloads each playlist sequentially with rate limiting.
6. Saves metadata to `download_metadata.txt` for future runs.

### Single playlist download — `download_single.sh`

Interactive mode for downloading a single playlist.

```bash
./download_single.sh
```

**Features:**

- Fuzzy-search your playlists with `fzf` (falls back to a filter prompt).
- TUI form to pick format (MP3 / FLAC / OGG) and output path.
- Navigate with arrow keys, Tab, Enter, Escape.
- Press `r` or `Ctrl-R` to refresh the playlist cache.

### Direct CLI usage — `spotify-dl`

Use the Rust binary directly for quick one-off downloads:

```bash
# Download a single track
./target/release/spotify-dl https://open.spotify.com/track/TRACK_ID

# Download a playlist as FLAC to a specific folder
./target/release/spotify-dl \
  -d ~/Music/MyPlaylist \
  -f flac \
  -t 1 \
  https://open.spotify.com/playlist/PLAYLIST_ID

# Force re-download with custom rate limit
./target/release/spotify-dl \
  -F \
  -r 30 \
  https://open.spotify.com/album/ALBUM_ID
```

#### CLI options

```
USAGE:
    spotify-dl [FLAGS] [OPTIONS] <tracks>...

FLAGS:
    -F, --force        Force download even if the file already exists
    -h, --help         Prints help information
    -V, --version      Prints version information

OPTIONS:
    -d, --destination <destination>  The directory where the songs will be downloaded
    -f, --format <format>            Output format: mp3, flac, ogg [default: flac]
    -t, --parallel <parallel>        Number of parallel downloads [default: 5]
    -r, --rate-limit <rate-limit>    Delay in seconds between each track download [default: 60]

ARGS:
    <tracks>...    Spotify URIs or URLs (songs, podcasts, playlists, or albums)
```

## 📁 Project Structure

```
├── .env.example          # Template for credentials
├── common.sh             # Shared config, .env loader, color definitions
├── start.sh              # Full library sync script
├── download_single.sh    # Interactive single-playlist downloader
├── spotify_code.php      # OAuth callback helper (self-hosted)
├── src/
│   ├── main.rs           # CLI entry point (structopt)
│   ├── download.rs       # Download orchestration & rate limiting
│   ├── track.rs          # Track resolution & metadata extraction
│   ├── session.rs        # Spotify session management
│   ├── log.rs            # Logging configuration
│   ├── utils.rs          # Utilities
│   ├── encoder/          # MP3 / FLAC / OGG encoding
│   └── stream/           # Audio stream handling
└── Cargo.toml
```

## 📄 License

spotify-dl is licensed under the MIT license. See [LICENSE](LICENSE).
