#!/bin/bash
# ──────────────────────────────────────────────────────────────────────
# Shared configuration — sourced by start.sh and download_single.sh
# ──────────────────────────────────────────────────────────────────────

SCRIPT_DIR="$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")"

# ──────────────────────────────────────────────────────────────────────
# Load .env file
# ──────────────────────────────────────────────────────────────────────
ENV_FILE="${SCRIPT_DIR}/.env"
if [[ ! -f "$ENV_FILE" ]]; then
    echo -e "\e[1;31mError: .env file not found at ${ENV_FILE}\e[0m"
    echo -e "\e[2mCopy .env.example to .env and fill in your values.\e[0m"
    exit 1
fi
set -a
source "$ENV_FILE"
set +a

# ──────────────────────────────────────────────────────────────────────
# Configuration (from .env)
# ──────────────────────────────────────────────────────────────────────
client_id="${SPOTIFY_CLIENT_ID:?Missing SPOTIFY_CLIENT_ID in .env}"
client_secret="${SPOTIFY_CLIENT_SECRET:?Missing SPOTIFY_CLIENT_SECRET in .env}"
redirect_uri="${SPOTIFY_REDIRECT_URI:?Missing SPOTIFY_REDIRECT_URI in .env}"
output_path="${OUTPUT_PATH:?Missing OUTPUT_PATH in .env}"
download_format="${DOWNLOAD_FORMAT:-mp3}"
rate_limit_secs="${RATE_LIMIT_SECS:-60}"

# Derived paths
spotify_dl="${SCRIPT_DIR}/target/release/spotify-dl"
folders_json="${SCRIPT_DIR}/spotifyfolders.json"
metadata_file="${SCRIPT_DIR}/download_metadata.txt"
playlist_cache="${SCRIPT_DIR}/playlist_names.cache"

# ──────────────────────────────────────────────────────────────────────
# Colors
# ──────────────────────────────────────────────────────────────────────
RED='\e[1;31m'
GREEN='\e[1;32m'
YELLOW='\e[1;33m'
CYAN='\e[1;36m'
BOLD='\e[1m'
DIM='\e[2m'
RESET='\e[0m'
