#!/bin/bash
set -uo pipefail

# ──────────────────────────────────────────────────────────────────────
# Load shared configuration
# ──────────────────────────────────────────────────────────────────────
source "$(dirname "$(readlink -f "$0")")/common.sh"

access_token=""
refresh_token=""
token_expires=3600

# ──────────────────────────────────────────────────────────────────────
# Load shared library (OAuth, playlist cache, utilities)
# ──────────────────────────────────────────────────────────────────────
source "$(dirname "$(readlink -f "$0")")/lib.sh"

declare -A playlist_name_cache

# Background token refresh loop (start.sh-specific, not needed by download_single.sh)
function start_token_refresh() {
    (
        while true; do
            sleep $((token_expires - 120))
            refresh_access_token 2>/dev/null || true
        done
    ) &
    REFRESH_PID=$!
    trap "kill $REFRESH_PID 2>/dev/null" EXIT
}

function resolve_playlist_name() {
    local uri="$1"
    local playlist_id="${uri##*:}"

    # Check cache first
    if [[ -n "${playlist_name_cache[$uri]+x}" ]]; then
        echo "${playlist_name_cache[$uri]}"
        return 0
    fi

    # Fetch from Spotify API
    local response
    response=$(curl -s "https://api.spotify.com/v1/playlists/${playlist_id}?fields=name" \
        -H "Authorization: Bearer $access_token" 2>/dev/null)

    local error_status
    error_status=$(echo "$response" | jq -r '.error.status // empty')

    if [[ "$error_status" == "401" ]]; then
        refresh_access_token
        response=$(curl -s "https://api.spotify.com/v1/playlists/${playlist_id}?fields=name" \
            -H "Authorization: Bearer $access_token" 2>/dev/null)
        error_status=$(echo "$response" | jq -r '.error.status // empty')
    fi

    if [[ "$error_status" == "429" ]]; then
        # Rate limited — wait and retry
        local retry_after
        retry_after=$(echo "$response" | jq -r '.retry_after // 5')
        sleep "$retry_after"
        response=$(curl -s "https://api.spotify.com/v1/playlists/${playlist_id}?fields=name" \
            -H "Authorization: Bearer $access_token" 2>/dev/null)
    fi

    local name
    name=$(echo "$response" | jq -r '.name // empty')

    if [[ -z "$name" ]]; then
        # Fallback: scrape name from the open.spotify.com page title
        local page_title
        page_title=$(curl -s "https://open.spotify.com/playlist/${playlist_id}" \
            -H "User-Agent: Mozilla/5.0" 2>/dev/null \
            | grep -oP '<title>\K[^<]+' | head -1)

        if [[ -n "$page_title" ]]; then
            # Title format is "playlist_name | Spotify Playlist" or similar
            name=$(echo "$page_title" | sed 's/ | Spotify.*//;s/ - Spotify$//' | xargs)
        fi
    fi

    if [[ -z "$name" ]]; then
        # Last resort: use playlist ID as folder name
        name="playlist_${playlist_id}"
    fi

    name=$(clean_filename "$name")
    playlist_name_cache["$uri"]="$name"
    echo "$name"
}

# ──────────────────────────────────────────────────────────────────────
# Folder hierarchy: run spotifyfolders + walk tree
# ──────────────────────────────────────────────────────────────────────
function refresh_folders() {
    echo -e "${CYAN}ℹ Fetching folder hierarchy from Spotify client...${RESET}"
    spotifyfolders > "$folders_json" 2>/dev/null

    if [[ ! -s "$folders_json" ]]; then
        echo -e "${RED}Error: spotifyfolders produced no output. Is Spotify running?${RESET}"
        exit 1
    fi

    local count
    count=$(python3 -c "
import json, sys
data = json.load(open('$folders_json'))
def count(n):
    if n.get('type')=='playlist': return 1
    return sum(count(c) for c in n.get('children',[]))
print(count(data))
")
    echo -e "${GREEN}✓ Found ${count} playlists in folder hierarchy${RESET}"
}

# Walk the JSON tree and output: uri|folder_path
# Folder path is the filesystem path relative to output_path
function build_playlist_map() {
    python3 -c "
import json, sys, os

data = json.load(open('$folders_json'))
output_path = '$output_path'

def walk(node, path_parts):
    t = node.get('type', '')
    if t == 'playlist':
        uri = node.get('uri', '')
        if uri:
            folder = os.path.join(*path_parts) if path_parts else ''
            print(f'{uri}|{folder}')
    elif t == 'folder':
        name = node.get('name', '')
        child_parts = path_parts + [name] if name else path_parts
        for child in node.get('children', []):
            walk(child, child_parts)

walk(data, [])
"
}

# ──────────────────────────────────────────────────────────────────────
# Download: process each playlist
# ──────────────────────────────────────────────────────────────────────
function process_playlist() {
    local uri="$1"
    local folder_path="$2"
    local playlist_name="$3"
    local dest_dir

    if [[ -n "$folder_path" ]]; then
        dest_dir="${output_path}/${folder_path}/${playlist_name}"
    else
        dest_dir="${output_path}/${playlist_name}"
    fi

    # Create destination directory
    mkdir -p "$dest_dir"

    echo -e "  ${BOLD}→ ${playlist_name}${RESET}"
    echo -e "    📂 ${dest_dir}"

    # Run spotify-dl with -t 1 (sequential) to avoid session concurrency issues
    # Multiple parallel Player instances on the same session cause false "unavailable" errors
    if "$spotify_dl" -d "$dest_dir" -f "$download_format" -t 1 -r "$rate_limit_secs" "$uri"; then
        # Update metadata file
        local meta_entry="${uri}|${dest_dir}"
        if ! grep -qF "$uri" "$metadata_file" 2>/dev/null; then
            echo "$meta_entry" >> "$metadata_file"
        else
            # Update path if it changed
            sed -i "s|^${uri}|.*$|${meta_entry}|" "$metadata_file" 2>/dev/null || true
        fi
        echo -e "    ${GREEN}✓ Done${RESET}\n"
    else
        echo -e "    ${RED}✗ Failed${RESET}\n"
    fi
}

# ──────────────────────────────────────────────────────────────────────
# Main
# ──────────────────────────────────────────────────────────────────────
echo -e "\n${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}"
echo -e "${BOLD}  Spotify Library Sync${RESET}"
echo -e "${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}\n"

# Step 1: Refresh folder hierarchy
refresh_folders

# Step 2: Authenticate with Spotify Web API
echo ""
load_playlist_cache
request_authorization
start_token_refresh

# Step 3: Batch-fetch playlist names from library, then resolve remaining
fetch_all_playlist_names

echo -e "\n${CYAN}ℹ Resolving playlist names...${RESET}"

declare -a download_queue=()
resolved=0

# Build the playlist map first so we know the total
playlist_map_file=$(mktemp)
build_playlist_map > "$playlist_map_file"
total=$(wc -l < "$playlist_map_file")

echo -e "  Found ${total} playlists to resolve\n"

while IFS='|' read -r uri folder_path; do
    resolved=$((resolved + 1))
    playlist_name=$(resolve_playlist_name "$uri" 2>/dev/null) || playlist_name=""
    if [[ -n "$playlist_name" ]]; then
        download_queue+=("${uri}|${folder_path}|${playlist_name}")
        echo -ne "\r  Resolved ${resolved}/${total} playlists..."
    else
        echo -e "\n  ${YELLOW}⚠ Could not resolve: ${uri}${RESET}"
    fi
    # Small delay to avoid rate limiting
    sleep 0.05
done < "$playlist_map_file"
rm -f "$playlist_map_file"

# Save resolved names to cache
save_playlist_cache
echo -e "\r  ${GREEN}✓ Resolved ${resolved}/${total} playlist names${RESET}"

# Step 4: Download all playlists
echo -e "\n${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}"
echo -e "${BOLD}  Downloading ${#download_queue[@]} playlists (format: ${download_format})${RESET}"
echo -e "${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}\n"

completed=0
failed=0

for entry in "${download_queue[@]}"; do
    IFS='|' read -r uri folder_path playlist_name <<< "$entry"
    completed=$((completed + 1))
    echo -e "${CYAN}[${completed}/${#download_queue[@]}]${RESET} ${folder_path:+${folder_path}/}${BOLD}${playlist_name}${RESET}"
    process_playlist "$uri" "$folder_path" "$playlist_name"
done

# Step 5: Summary
echo -e "\n${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}"
echo -e "${GREEN}  Sync complete: ${completed} playlists processed${RESET}"
echo -e "${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}\n"
