#!/bin/bash
set -uo pipefail

source "$(dirname "$(readlink -f "$0")")/common.sh"

# ──────────────────────────────────────────────────────────────────────
# Helpers
# ──────────────────────────────────────────────────────────────────────
function read_key() {
    local key
    IFS= read -rsn1 key
    if [[ "$key" == $'\e' ]]; then
        local s1 s2
        IFS= read -rsn1 -t 0.05 s1
        [[ -z "$s1" ]] && { echo escape; return; }
        IFS= read -rsn1 -t 0.05 s2
        case "${s1}${s2}" in
            '[A') echo up;; '[B') echo down;;
            '[C') echo right;; '[D') echo left;;
            *) echo unknown;;
        esac
    elif [[ "$key" == $'\t' ]]; then echo tab
    elif [[ "$key" == "" ]]; then echo enter
    elif [[ "$key" == "r" || "$key" == "R" ]]; then echo refresh
    else echo other; fi
}

function cache_age() {
    if [[ ! -f "$playlist_cache" ]]; then echo "missing"; return; fi
    local now mod diff
    now=$(date +%s)
    mod=$(stat -c %Y "$playlist_cache" 2>/dev/null || echo "$now")
    diff=$((now - mod))
    if   [[ $diff -lt 60 ]];    then echo "just now"
    elif [[ $diff -lt 3600 ]];  then echo "$((diff / 60))m ago"
    elif [[ $diff -lt 86400 ]]; then echo "$((diff / 3600))h ago"
    else echo "$((diff / 86400))d ago"; fi
}

function load_cache() {
    uris=(); names=()
    [[ ! -f "$playlist_cache" ]] && return
    while IFS='|' read -r uri name; do
        [[ -z "$uri" || -z "$name" ]] && continue
        uris+=("$uri"); names+=("$name")
    done < "$playlist_cache"
}

function refresh_cache() {
    echo -e "\n  ${DIM}Refreshing playlist cache...${RESET}"

    # Source OAuth and fetch functions from start.sh
    local start_sh
    start_sh="$(dirname "$(readlink -f "$0")")/start.sh"

    if [[ ! -f "$start_sh" ]]; then
        echo -e "  ${RED}✗ start.sh not found${RESET}\n"
        return 1
    fi

    # Extract needed functions from start.sh
    eval "$(sed -n '/^function request_authorization/,/^}/p' "$start_sh")"
    eval "$(sed -n '/^function refresh_access_token/,/^}/p' "$start_sh")"
    eval "$(sed -n '/^function clean_filename/,/^}/p' "$start_sh")"
    eval "$(sed -n '/^function fetch_all_playlist_names/,/^}/p' "$start_sh")"
    eval "$(sed -n '/^function load_playlist_cache/,/^}/p' "$start_sh")"

    # Global so eval'd functions can access it (bash scoping)
    declare -gA playlist_name_cache

    access_token=""
    refresh_token=""
    token_expires=3600

    load_playlist_cache
    request_authorization
    fetch_all_playlist_names

    # Save cache directly (don't rely on eval'd save function)
    > "$playlist_cache"
    for uri in "${!playlist_name_cache[@]}"; do
        echo "${uri}|${playlist_name_cache[$uri]}" >> "$playlist_cache"
    done
    touch "$playlist_cache"

    echo -e "  ${GREEN}✓ Cache updated (${#playlist_name_cache[@]} playlists)${RESET}\n"

    # Reload into arrays
    load_cache
}

# ──────────────────────────────────────────────────────────────────────
# Preflight
# ──────────────────────────────────────────────────────────────────────
[[ ! -x "$spotify_dl" ]] && { echo -e "\n${RED}  ✗ spotify-dl not found.${RESET}\n"; exit 1; }

# ──────────────────────────────────────────────────────────────────────
# Load cache + show header
# ──────────────────────────────────────────────────────────────────────
declare -a uris=() names=()
load_cache

if [[ ${#names[@]} -eq 0 ]]; then
    echo -e "\n  ${GREEN}spotify-dl${RESET} ${DIM}/ single download${RESET}"
    echo -e "  ${DIM}Cache: ${YELLOW}empty or missing${RESET}"
    echo -e "\n  ${DIM}Press ${BOLD}r${DIM} to refresh cache, or run start.sh first${RESET}\n"
    while true; do
        key=$(read_key)
        case "$key" in
            refresh) refresh_cache; [[ ${#names[@]} -gt 0 ]] && break || continue ;;
            escape)  echo -e "  ${YELLOW}Cancelled.${RESET}\n"; exit 0 ;;
            *)       continue ;;
        esac
    done
fi

echo -e "\n  ${GREEN}spotify-dl${RESET} ${DIM}/ single download / ${#names[@]} playlists / cache $(cache_age)${RESET}"
echo -e "  ${DIM}r = refresh cache${RESET}\n"

# ──────────────────────────────────────────────────────────────────────
# Playlist selection
# ──────────────────────────────────────────────────────────────────────
selected_index=""

if command -v fzf &>/dev/null; then
    while true; do
        fzf_input=""
        for i in "${!names[@]}"; do
            fzf_input+="$(printf '%d\t%s' $((i + 1)) "${names[$i]}")"$'\n'
        done
        selection=$(echo "$fzf_input" | fzf \
            --height=80% --reverse \
            --prompt="  > " --pointer=">" \
            --header="  Select a playlist  |  ESC cancel  |  ctrl-r refresh" --header-first \
            --color='fg:7,bg:-1,hl:2,fg+:15,bg+:236,hl+:10' \
            --color='info:2,prompt:2,pointer:2,marker:2,spinner:2,header:8' \
            --delimiter='\t' --with-nth=2 --no-multi --no-scrollbar --border=none \
            --expect=ctrl-r)

        key_pressed=$(echo "$selection" | head -1)
        chosen=$(echo "$selection" | tail -1)

        if [[ "$key_pressed" == "ctrl-r" ]]; then
            refresh_cache
            echo -e "  ${GREEN}spotify-dl${RESET} ${DIM}/ ${#names[@]} playlists / cache $(cache_age)${RESET}\n"
            continue
        elif [[ -z "$chosen" ]]; then
            echo -e "  ${YELLOW}Cancelled.${RESET}\n"; exit 0
        fi

        selected_index=$(echo "$chosen" | cut -f1)
        selected_index=$((selected_index - 1))
        break
    done
else
    echo -ne "  ${DIM}Filter (r=refresh):${RESET} "; read -r filter

    if [[ "$filter" == "r" || "$filter" == "R" ]]; then
        refresh_cache
        echo -ne "  ${DIM}Filter:${RESET} "; read -r filter
    fi

    declare -a filtered_indices=()
    if [[ -n "$filter" ]]; then
        for i in "${!names[@]}"; do
            echo "${names[$i]}" | grep -qi "$filter" && filtered_indices+=("$i")
        done
    else
        for i in "${!names[@]}"; do filtered_indices+=("$i"); done
    fi
    [[ ${#filtered_indices[@]} -eq 0 ]] && { echo -e "\n  ${YELLOW}No matches.${RESET}\n"; exit 0; }

    echo ""
    for j in "${!filtered_indices[@]}"; do
        printf "  ${CYAN}%3d${RESET}  %s\n" $((j + 1)) "${names[${filtered_indices[$j]}]}"
    done
    echo ""; echo -ne "  ${DIM}#:${RESET} "; read -r choice
    if [[ -z "$choice" ]] || ! [[ "$choice" =~ ^[0-9]+$ ]] \
       || [[ "$choice" -lt 1 ]] || [[ "$choice" -gt ${#filtered_indices[@]} ]]; then
        echo -e "  ${RED}Invalid selection.${RESET}\n"; exit 1
    fi
    selected_index="${filtered_indices[$((choice - 1))]}"
fi

# ──────────────────────────────────────────────────────────────────────
# Interactive form
# ──────────────────────────────────────────────────────────────────────
playlist_name="${names[$selected_index]}"
uri="${uris[$selected_index]}"

formats=(mp3 flac ogg)
format_idx=0
for i in "${!formats[@]}"; do
    [[ "${formats[$i]}" == "$download_format" ]] && format_idx=$i
done

full_path="${output_path}/${playlist_name}"
current_field=0   # 0=format  1=path  2=download
FORM_LINES=7

function print_form() {
    printf '\e[J'
    echo -e "  ${BOLD}${playlist_name}${RESET}"
    echo ""

    local pf="   "
    [[ $current_field -eq 0 ]] && pf=" ${GREEN}>${RESET}"
    echo -ne "${pf} Format  "
    for i in "${!formats[@]}"; do
        if [[ $i -eq $format_idx ]]; then
            echo -ne "  ${BOLD}[${formats[$i]}]${RESET}"
        else
            echo -ne "   ${DIM}${formats[$i]}${RESET} "
        fi
    done
    printf '\e[K\n'

    local pp="   "
    [[ $current_field -eq 1 ]] && pp=" ${GREEN}>${RESET}"
    echo -e "${pp} Path    ${full_path}\e[K"

    echo ""

    if [[ $current_field -eq 2 ]]; then
        echo -ne "  ${GREEN}> Download${RESET}"
    else
        echo -ne "    ${DIM}Download${RESET}"
    fi
    echo -e "    ${DIM}arrows/tab  enter  esc${RESET}\e[K"

    echo ""
}

echo ""
print_form

while true; do
    key=$(read_key)
    case "$key" in
        up)
            current_field=$(( (current_field - 1 + 3) % 3 ))
            printf '\e[%dA' $FORM_LINES; print_form ;;
        down|tab)
            current_field=$(( (current_field + 1) % 3 ))
            printf '\e[%dA' $FORM_LINES; print_form ;;
        left)
            if [[ $current_field -eq 0 ]]; then
                format_idx=$(( (format_idx - 1 + ${#formats[@]}) % ${#formats[@]} ))
                printf '\e[%dA' $FORM_LINES; print_form
            fi ;;
        right)
            if [[ $current_field -eq 0 ]]; then
                format_idx=$(( (format_idx + 1) % ${#formats[@]} ))
                printf '\e[%dA' $FORM_LINES; print_form
            fi ;;
        enter)
            if [[ $current_field -eq 0 ]]; then
                format_idx=$(( (format_idx + 1) % ${#formats[@]} ))
                printf '\e[%dA' $FORM_LINES; print_form
            elif [[ $current_field -eq 1 ]]; then
                printf '\e[4A\e[2K\r'
                echo -ne " ${GREEN}>${RESET} Path    "
                read -e -i "$full_path" new_path
                [[ -n "$new_path" ]] && full_path="$new_path"
                printf '\e[4A'
                print_form
            elif [[ $current_field -eq 2 ]]; then
                break
            fi ;;
        escape)
            echo -e "  ${YELLOW}Cancelled.${RESET}\n"; exit 0 ;;
    esac
done

# ──────────────────────────────────────────────────────────────────────
# Download
# ──────────────────────────────────────────────────────────────────────
download_format="${formats[$format_idx]}"
dest_dir="$full_path"

mkdir -p "$dest_dir"

echo -e "  ${DIM}Downloading ${BOLD}${playlist_name}${RESET}${DIM} as ${download_format}${RESET}\n"

if "$spotify_dl" -d "$dest_dir" -f "$download_format" -t 1 "$uri"; then
    meta_entry="${uri}|${dest_dir}"
    if ! grep -qF "$uri" "$metadata_file" 2>/dev/null; then
        echo "$meta_entry" >> "$metadata_file"
    else
        sed -i "s|^${uri}|.*|${meta_entry}|" "$metadata_file" 2>/dev/null || true
    fi
    echo -e "\n  ${GREEN}✓ Done${RESET} ${DIM}${dest_dir}${RESET}\n"
else
    echo -e "\n  ${RED}✗ Failed${RESET} ${DIM}${playlist_name}${RESET}\n"
    exit 1
fi
