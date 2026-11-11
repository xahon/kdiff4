#!/usr/bin/env bash
set -euo pipefail

installer=$(cd "$(dirname "$0")/../scripts" && pwd)/install.sh
HOME=$(mktemp -d)
trap 'rm -f -- "$HOME"/.local/bin/* "$HOME"/.local/bin/.kdiff4.* "$HOME"/.bashrc "$HOME"/.profile "$HOME"/.zshrc; rmdir "$HOME"/.local/bin "$HOME"/.local "$HOME"' EXIT
export HOME SHELL=/bin/bash

uname() {
    case "$1" in
        -s) echo "$test_os" ;;
        -m) echo "$test_arch" ;;
    esac
}

curl() {
    [[ "$2" == "https://github.com/xahon/kdiff4/releases/latest/download/$test_asset" ]] || return 1
    printf '#!/bin/sh\necho %s\n' "$test_asset" > "$4"
    [[ "${test_fail:-}" != yes ]]
}

export -f uname curl
export test_os=Linux test_arch=x86_64 test_asset=kdiff4-linux-x86_64
bash "$installer"
bash "$installer"
[[ -x "$HOME/.local/bin/kdiff4" ]]
[[ $("$HOME/.local/bin/kdiff4") == "$test_asset" ]]
for profile in .bashrc .profile; do
    [[ $(grep -Fc 'export PATH=' "$HOME/$profile") == 1 ]]
    ( source "$HOME/$profile"; [[ $(command -v kdiff4) == "$HOME/.local/bin/kdiff4" ]] )
done

export test_os=Darwin test_arch=arm64 test_asset=kdiff4-macos-aarch64 SHELL=/bin/zsh
export test_fail=yes
if bash "$installer"; then
    echo 'Expected a failed download' >&2
    exit 1
fi
[[ $("$HOME/.local/bin/kdiff4") == kdiff4-linux-x86_64 ]]
[[ $(find "$HOME/.local/bin" -type f | wc -l) -eq 1 ]]
unset test_fail

bash "$installer"
[[ $("$HOME/.local/bin/kdiff4") == "$test_asset" ]]
[[ $(grep -Fc 'export PATH=' "$HOME/.zshrc") == 1 ]]
