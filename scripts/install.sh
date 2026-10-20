#!/bin/sh
# Installs `hb` and `hebra-server` from the latest GitHub release (or
# HEBRA_VERSION), after checking the archive against the release's SHA256SUMS.
#
#   curl -fsSL https://raw.githubusercontent.com/s-pl/hebra/main/scripts/install.sh | sh
#
# HEBRA_INSTALL_DIR chooses where (default ~/.local/bin).
set -eu

REPO="s-pl/hebra"
DIR="${HEBRA_INSTALL_DIR:-$HOME/.local/bin}"

case "$(uname -s)" in
  Linux) os="unknown-linux-gnu" ;;
  Darwin) os="apple-darwin" ;;
  *) echo "unsupported system: $(uname -s) (on Windows use install.ps1)" >&2; exit 1 ;;
esac
case "$(uname -m)" in
  x86_64 | amd64) arch="x86_64" ;;
  arm64 | aarch64) arch="aarch64" ;;
  *) echo "unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac

version="${HEBRA_VERSION:-}"
if [ -z "$version" ]; then
  version=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" |
    sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1)
fi
[ -n "$version" ] || { echo "could not find the latest release" >&2; exit 1; }

name="hebra-$version-$arch-$os"
base="https://github.com/$REPO/releases/download/$version"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

echo "Downloading Hebra $version for $arch-$os"
curl -fsSL -o "$tmp/$name.tar.gz" "$base/$name.tar.gz"
curl -fsSL -o "$tmp/SHA256SUMS" "$base/SHA256SUMS"
expected=$(grep " $name.tar.gz\$" "$tmp/SHA256SUMS" | cut -d' ' -f1)
if command -v sha256sum >/dev/null; then
  actual=$(sha256sum "$tmp/$name.tar.gz" | cut -d' ' -f1)
else
  actual=$(shasum -a 256 "$tmp/$name.tar.gz" | cut -d' ' -f1)
fi
[ -n "$expected" ] && [ "$expected" = "$actual" ] || { echo "checksum mismatch for $name.tar.gz" >&2; exit 1; }

tar xzf "$tmp/$name.tar.gz" -C "$tmp"
mkdir -p "$DIR"
install -m 755 "$tmp/$name/hb" "$tmp/$name/hebra-server" "$DIR/"
echo "Installed hb and hebra-server to $DIR"
case ":$PATH:" in
  *":$DIR:"*) ;;
  *) echo "Add $DIR to your PATH to use them." ;;
esac
"$DIR/hb" --version
