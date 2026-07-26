#!/usr/bin/env bash
# Minimal installer for prebuilt heim binaries from GitHub Releases.
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/aiatsuk/heim/main/scripts/install.sh | bash
#   HEIM_VERSION=0.1.0 bash scripts/install.sh
set -euo pipefail

REPO="${HEIM_REPO:-aiatsuk/heim}"
VERSION="${HEIM_VERSION:-latest}"
PREFIX="${HEIM_PREFIX:-/usr/local/bin}"

need() { command -v "$1" >/dev/null 2>&1 || { echo "need $1" >&2; exit 1; }; }
need curl
need tar
need uname

os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
  Darwin) os_tag="apple-darwin" ;;
  Linux) os_tag="unknown-linux-gnu" ;;
  *) echo "unsupported OS: $os" >&2; exit 1 ;;
esac
case "$arch" in
  arm64|aarch64) arch_tag="aarch64" ;;
  x86_64|amd64) arch_tag="x86_64" ;;
  *) echo "unsupported arch: $arch" >&2; exit 1 ;;
esac
target="${arch_tag}-${os_tag}"

if [[ "$VERSION" == "latest" ]]; then
  need grep
  VERSION="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
    | grep -o '"tag_name": *"[^"]*"' | head -1 | sed 's/.*"v\{0,1\}\([^"]*\)"/\1/;s/^v//')"
fi
VERSION="${VERSION#v}"

asset="heim-${VERSION}-${target}.tar.gz"
url="https://github.com/${REPO}/releases/download/v${VERSION}/${asset}"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "downloading $url"
curl -fsSL "$url" -o "$tmp/$asset"
tar -C "$tmp" -xzf "$tmp/$asset"
install -m 755 "$tmp/heim" "$PREFIX/heim"
echo "installed: $PREFIX/heim ($("$PREFIX/heim" -V 2>/dev/null || echo "v$VERSION"))"
