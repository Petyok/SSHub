#!/bin/sh
# phone-refresh.sh <poc/phone worktree> <commit>: pin the page the e2e phone runs (byte for byte, with its fingerprint).
set -eu
wt=$1; rev=$2
here=$(cd "$(dirname "$0")" && pwd)
git -C "$wt" show "$rev:assets/companion/sshub-companion.html" > "$here/phone-page/sshub-companion.html"
git -C "$wt" show "$rev:assets/companion/FINGERPRINT" > "$here/phone-page/FINGERPRINT"
printf 'poc/phone %s (assets/companion/sshub-companion.html and FINGERPRINT, byte for byte)\nRefresh: deploy/e2e/phone-refresh.sh <phone-worktree> <commit>\n' "$rev" > "$here/phone-page/SOURCE"
echo "pinned $rev: $(cat "$here/phone-page/FINGERPRINT")"
