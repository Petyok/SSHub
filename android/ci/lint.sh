#!/bin/sh
# Source rules that fail the build. Run from anywhere; exits 1 on any finding.
set -u
src=$(cd "$(dirname "$0")/../app/src" && pwd)
rc=0
fail() { echo "lint: $1" >&2; rc=1; }

# Colours only in the theme.
hits=$(grep -rnF 'Color(0x' "$src" --include='*.kt' | grep -v '/ui/theme/Theme.kt' || true)
[ -z "$hits" ] || { printf '%s\n' "$hits" >&2; fail "raw Color(0x outside ui/theme/Theme.kt"; }

# D4: no truncation of text.
hits=$(grep -rnE 'Ellipsis|maxLines|TextOverflow' "$src"/main/java/io/github/petyok/sshub/ui --include='*.kt' 2>/dev/null || true)
[ -z "$hits" ] || { printf '%s\n' "$hits" >&2; fail "Ellipsis/maxLines/TextOverflow under ui"; }

# No logging where secrets live.
hits=$(grep -rnE '(^|[^A-Za-z0-9_.])Log\.[a-z]+\(|android\.util\.Log' "$src"/main/java/io/github/petyok/sshub/platform "$src"/main/java/io/github/petyok/sshub/core --include='*.kt' 2>/dev/null || true)
[ -z "$hits" ] || { printf '%s\n' "$hits" >&2; fail "Log. under platform/ or core/"; }

# Words and shares never go to saved instance state.
for f in $(grep -rlF 'rememberSaveable' "$src" --include='*.kt' 2>/dev/null); do
    near=$(grep -nF -B3 -A3 'rememberSaveable' "$f" | grep -iE 'word|mnemonic|phrase|share' || true)
    [ -z "$near" ] || { printf '%s: %s\n' "$f" "$near" >&2; fail "rememberSaveable near words or shares"; }
done

[ "$rc" -eq 0 ] && echo "lint ok"
exit "$rc"
