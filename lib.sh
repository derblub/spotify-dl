#!/bin/bash
# ──────────────────────────────────────────────────────────────────────
# Shared library — sourced by start.sh and download_single.sh
# Requires common.sh to be sourced first (provides config variables).
# ──────────────────────────────────────────────────────────────────────

# ──────────────────────────────────────────────────────────────────────
# Utility: clean invalid filesystem characters
# ──────────────────────────────────────────────────────────────────────
function clean_filename() {
    local input="$1"
    # Remove characters invalid on most filesystems
    echo "$input" | sed 's/[<>:"\/\\|?*]//g' | sed 's/[[:cntrl:]]//g' | sed 's/\.\.\./…/g' | sed 's/[[:space:]]*$//'
}

# ──────────────────────────────────────────────────────────────────────
# OAuth: Web API authentication (for playlist name resolution)
# ──────────────────────────────────────────────────────────────────────
function request_authorization() {
    local scope="playlist-read-private"
    local url="https://accounts.spotify.com/authorize?client_id=${client_id}&response_type=code&redirect_uri=${redirect_uri}&scope=${scope}"
    echo -e "\n${CYAN}Open this URL to authorize:${RESET}"
    echo -e "${BOLD}${url}${RESET}\n"
    read -r -p "Paste the authorization code: " code

    if [[ -z "$code" ]]; then
        echo -e "${RED}Error: No code provided.${RESET}"
        exit 1
    fi

    local response
    response=$(curl -s -X POST "https://accounts.spotify.com/api/token" \
        -d "grant_type=authorization_code" \
        -d "code=$code" \
        -d "redirect_uri=$redirect_uri" \
        -d "client_id=$client_id" \
        -d "client_secret=$client_secret" \
        -H "Content-Type: application/x-www-form-urlencoded")

    access_token=$(echo "$response" | jq -r '.access_token // empty')
    refresh_token=$(echo "$response" | jq -r '.refresh_token // empty')
    token_expires=$(echo "$response" | jq -r '.expires_in // 3600')

    if [[ -z "$access_token" ]]; then
        echo -e "${RED}Error: Failed to obtain access token. Response: $response${RESET}"
        exit 1
    fi

    echo -e "${GREEN}✓ Obtained access token (valid for ${token_expires}s)${RESET}"
}

function refresh_access_token() {
    local response
    response=$(curl -s -X POST "https://accounts.spotify.com/api/token" \
        -d "grant_type=refresh_token" \
        -d "refresh_token=$refresh_token" \
        -d "client_id=$client_id" \
        -d "client_secret=$client_secret" \
        -H "Content-Type: application/x-www-form-urlencoded")

    access_token=$(echo "$response" | jq -r '.access_token // empty')
    local new_refresh=$(echo "$response" | jq -r '.refresh_token // empty')
    [[ -n "$new_refresh" ]] && refresh_token="$new_refresh"
    token_expires=$(echo "$response" | jq -r '.expires_in // 3600')

    if [[ -z "$access_token" ]]; then
        echo -e "${RED}Error: Failed to refresh token. Response: $response${RESET}"
        return 1
    fi

    echo -e "${GREEN}✓ Token refreshed (valid for ${token_expires}s)${RESET}"
}

# ──────────────────────────────────────────────────────────────────────
# Playlist name cache
# ──────────────────────────────────────────────────────────────────────
function load_playlist_cache() {
    # Load from dedicated name cache
    if [[ -f "$playlist_cache" ]]; then
        while IFS='|' read -r uri name; do
            playlist_name_cache["$uri"]="$name"
        done < "$playlist_cache"
    fi

    # Also load names from download_metadata.txt (URI|path format)
    # The last path component is the playlist name
    if [[ -f "$metadata_file" ]]; then
        while IFS='|' read -r uri path; do
            if [[ -z "${playlist_name_cache[$uri]+x}" && -n "$path" ]]; then
                local name
                name=$(basename "$path")
                if [[ -n "$name" && "$name" != playlist_* ]]; then
                    playlist_name_cache["$uri"]="$name"
                fi
            fi
        done < "$metadata_file"
    fi

    if [[ ${#playlist_name_cache[@]} -gt 0 ]]; then
        echo -e "${CYAN}ℹ Loaded ${#playlist_name_cache[@]} cached playlist names${RESET}"
    fi
}

function save_playlist_cache() {
    > "$playlist_cache"
    for uri in "${!playlist_name_cache[@]}"; do
        echo "${uri}|${playlist_name_cache[$uri]}" >> "$playlist_cache"
    done
}

# Pre-populate name cache by fetching all playlists from the user's library
# This catches Spotify-generated playlists that return 404 on individual lookup
function fetch_all_playlist_names() {
    echo -e "${CYAN}ℹ Fetching playlist names from your library...${RESET}"
    local limit=50
    local offset=0
    local total=0
    local fetched=0

    while true; do
        local response
        response=$(curl -s "https://api.spotify.com/v1/me/playlists?limit=${limit}&offset=${offset}" \
            -H "Authorization: Bearer $access_token" 2>/dev/null)

        local error_status
        error_status=$(echo "$response" | jq -r '.error.status // empty')

        if [[ "$error_status" == "401" ]]; then
            refresh_access_token
            response=$(curl -s "https://api.spotify.com/v1/me/playlists?limit=${limit}&offset=${offset}" \
                -H "Authorization: Bearer $access_token" 2>/dev/null)
        fi

        if [[ $offset -eq 0 ]]; then
            total=$(echo "$response" | jq -r '.total // 0')
            echo -e "  Found ${total} playlists in your library"
        fi

        # Extract uri→name pairs and populate cache
        local count
        count=$(echo "$response" | jq -r '.items | length')

        if [[ "$count" -eq 0 ]] || [[ "$count" == "null" ]]; then
            break
        fi

        while IFS=$'\t' read -r uri name; do
            if [[ -n "$uri" && -n "$name" ]]; then
                local clean_name
                clean_name=$(clean_filename "$name")
                playlist_name_cache["$uri"]="$clean_name"
                fetched=$((fetched + 1))
            fi
        done < <(echo "$response" | jq -r '.items[] | [.uri, .name] | @tsv')

        echo -ne "\r  Fetched ${fetched}/${total} playlist names..."

        offset=$((offset + limit))
        if [[ $offset -ge $total ]]; then
            break
        fi

        sleep 0.1
    done

    echo -e "\r  ${GREEN}✓ Fetched ${fetched} playlist names from library${RESET}"
}
