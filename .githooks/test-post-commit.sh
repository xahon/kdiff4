#!/bin/sh
# Runs the post-commit hook in a throwaway repository: sh .githooks/test-post-commit.sh
hook_dir=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
failures=0

check() {
    if [ "$2" = "$3" ]; then
        echo "ok   $1"
    else
        echo "FAIL $1: expected [$2], got [$3]"
        failures=$((failures + 1))
    fi
}

write_manifest() {
    printf '[workspace]\nmembers = []\n\n[workspace.package]\nversion = "%s"\nedition = "2024"\n\n[workspace.dependencies]\nfoo = { version = "9.9.9" }\n' "$1" > Cargo.toml
}

cd "$work" && git init -q -b master . && git config user.email t@example.com && git config user.name t && git config core.hooksPath "$hook_dir"

write_manifest 0.1.0
git add . && git commit -q -m first
check "first commit is tagged" "v0.1.0" "$(git tag -l)"
first_commit=$(git rev-parse HEAD)

echo a > file.txt && git add . && git commit -q -m unrelated
check "commit without a version change adds no tag" "v0.1.0" "$(git tag -l)"

write_manifest 0.2.0
git commit -q -am bump
check "version bump is tagged" "v0.1.0 v0.2.0" "$(git tag -l | tr '\n' ' ' | sed 's/ $//')"
check "tag is annotated" "tag" "$(git cat-file -t v0.2.0)"
check "tag points at the bump" "$(git rev-parse HEAD)" "$(git rev-parse 'v0.2.0^{commit}')"

printf '\n[workspace.dependencies]\nbar = { version = "1.0.0" }\n' >> Cargo.toml
git commit -q -am "dependency edit"
check "dependency versions are ignored" "v0.1.0 v0.2.0" "$(git tag -l | tr '\n' ' ' | sed 's/ $//')"

git checkout -q -b feature/x && write_manifest 0.3.0 && git commit -q -am "bump on a feature branch"
check "other branches are not tagged" "v0.1.0 v0.2.0" "$(git tag -l | tr '\n' ' ' | sed 's/ $//')"

git checkout -q master && write_manifest 0.2.0 && echo b >> file.txt && git commit -q -am "back to 0.2.0"
write_manifest 0.1.0 && git commit -q -am "down to 0.1.0 where the tag exists"
check "existing tags are never moved" "$first_commit" "$(git rev-parse 'v0.1.0^{commit}')"

exit $failures
