#!/usr/bin/env bash
set -euo pipefail

case "$(uname -s):$(uname -m)" in
    Linux:x86_64) asset=kdiff4-linux-x86_64 ;;
    Darwin:arm64) asset=kdiff4-macos-aarch64 ;;
    *)
        echo 'Kdiff4 releases support Linux x86-64 and Apple silicon macOS with this installer.' >&2
        exit 1
        ;;
esac

bin_dir="$HOME/.local/bin"
mkdir -p "$bin_dir"
download=$(mktemp "$bin_dir/.kdiff4.XXXXXXXX")
trap 'rm -f -- "$download"' EXIT
curl -fSL "https://github.com/xahon/kdiff4/releases/latest/download/$asset" -o "$download"
chmod +x "$download"
mv -f -- "$download" "$bin_dir/kdiff4"

path_line='case ":$PATH:" in *":$HOME/.local/bin:"*) ;; *) export PATH="$HOME/.local/bin:$PATH" ;; esac'

add_to_path() {
    local profile=$1
    if ! grep -Fxq -- "$path_line" "$profile" 2>/dev/null; then
        printf '\n%s\n' "$path_line" >> "$profile"
    fi
}

shell=${SHELL:-bash}
case "${shell##*/}" in
    zsh) add_to_path "${ZDOTDIR:-$HOME}/.zshrc" ;;
    bash|'')
        add_to_path "$HOME/.bashrc"
        for profile in .bash_profile .bash_login .profile; do
            if [[ -f "$HOME/$profile" ]]; then
                break
            fi
        done
        add_to_path "$HOME/$profile"
        ;;
    *) printf 'Add %s to PATH in your shell settings.\n' "$bin_dir" ;;
esac

printf 'Installed %s/kdiff4. Open a new terminal to use it.\n' "$bin_dir"
