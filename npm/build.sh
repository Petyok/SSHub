#!/usr/bin/env bash
# Assemble, verify and optionally publish the npm packages for a released version.
#
# No binary is built here. The binaries are the tarballs the release workflow
# already attached to the `vX.Y.Z` GitHub release, so what npm serves is byte for
# byte what the release serves.
#
# Usage:
#   npm/build.sh                      # version from Cargo.toml, assemble + verify
#   npm/build.sh 0.10.0               # explicit version
#   npm/build.sh --publish            # assemble, verify, then publish to npm
#   TARBALL_DIR=dir npm/build.sh      # use tarballs from a directory instead of
#                                     # downloading them (used by CI, which has
#                                     # them as workflow artifacts already)
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

version=""
publish=0
for arg in "$@"; do
    case "$arg" in
        "") ;;
        --publish) publish=1 ;;
        -*)
            echo "unknown flag: $arg" >&2
            exit 2
            ;;
        *) version="$arg" ;;
    esac
done

if [ -z "$version" ]; then
    version="$(sed -n '/^\[package\]/,/^\[/ s/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
fi
[ -n "$version" ] || {
    echo "could not determine the version" >&2
    exit 1
}

dist="npm/dist"
# Release order matters: the wrapper is published last, after the packages its
# optionalDependencies point at exist. Published the other way round, npm
# resolves the wrapper to no binary at all and `npx sshub` fails for everyone who
# installs in that window.
packages=(sshub-linux-x64 sshub-darwin-arm64 sshub-darwin-x64 sshub)

# triple | package | os | cpu
targets=(
    "x86_64-unknown-linux-gnu|sshub-linux-x64|linux|x64"
    "aarch64-apple-darwin|sshub-darwin-arm64|darwin|arm64"
    "x86_64-apple-darwin|sshub-darwin-x64|darwin|x64"
)

echo "==> assembling npm packages for $version"
rm -rf "$dist"
mkdir -p "$dist/tarballs"

if [ -n "${TARBALL_DIR:-}" ]; then
    cp "$TARBALL_DIR"/sshub-*.tar.gz "$dist/tarballs/"
else
    gh release download "v$version" --pattern 'sshub-*.tar.gz' --dir "$dist/tarballs"
fi

for entry in "${targets[@]}"; do
    IFS='|' read -r triple pkg os cpu <<<"$entry"
    tarball="$dist/tarballs/sshub-v$version-$triple.tar.gz"
    [ -f "$tarball" ] || {
        echo "missing $tarball" >&2
        exit 1
    }
    mkdir -p "$dist/$pkg/bin"
    tar xzf "$tarball" -C "$dist/$pkg/bin" sshub
    chmod 755 "$dist/$pkg/bin/sshub"
    sed -e "s|@NAME@|$pkg|g" -e "s|@VERSION@|$version|g" \
        -e "s|@OS@|$os|g" -e "s|@CPU@|$cpu|g" \
        npm/platform/package.json.in >"$dist/$pkg/package.json"
    sed -e "s|@NAME@|$pkg|g" -e "s|@TRIPLE@|$triple|g" \
        npm/platform/README.md.in >"$dist/$pkg/README.md"
    cp LICENSE "$dist/$pkg/LICENSE"
    echo "    $pkg  ($triple)"
done

mkdir -p "$dist/sshub"
cp -r npm/wrapper/bin "$dist/sshub/bin"
chmod 755 "$dist/sshub/bin/sshub.js"
sed -e "s|@VERSION@|$version|g" npm/wrapper/package.json.in >"$dist/sshub/package.json"
cp npm/wrapper/README.md "$dist/sshub/README.md"
cp LICENSE "$dist/sshub/LICENSE"
echo "    sshub  (wrapper)"

echo "==> verifying"
host="$(uname -s)-$(uname -m)"
if [ "$host" = "Linux-x86_64" ]; then
    # Catches the packaging pointing at a different release than it claims.
    got="$("$dist/sshub-linux-x64/bin/sshub" --version)"
    [ "$got" = "sshub $version" ] || {
        echo "binary reports '$got', packaging says '$version'" >&2
        exit 1
    }
    # Mirror a real install so the shim's require.resolve has somewhere to look,
    # then drive the shim exactly the way `npx sshub` would.
    mkdir -p "$dist/node_modules"
    ln -sfn ../sshub-linux-x64 "$dist/node_modules/sshub-linux-x64"
    got="$(node "$dist/sshub/bin/sshub.js" --version)"
    [ "$got" = "sshub $version" ] || {
        echo "the shim did not reach the binary, it printed '$got'" >&2
        exit 1
    }
    echo "    binary and shim both report sshub $version"
else
    echo "    skipped the exec check: this host is $host, not Linux-x86_64"
fi

for pkg in "${packages[@]}"; do
    (cd "$dist/$pkg" && npm pack --dry-run)
done

if [ "$publish" = 0 ]; then
    echo "==> assembled in $dist, nothing published"
    echo "    publish order: ${packages[*]}"
    exit 0
fi

echo "==> publishing $version"
for pkg in "${packages[@]}"; do
    echo "    $pkg@$version"
    (cd "$dist/$pkg" && npm publish --access public)
done
echo "==> published: npx sshub@$version"
