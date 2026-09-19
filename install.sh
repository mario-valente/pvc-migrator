#!/usr/bin/env bash
# Downloads the latest pvc-migrator release (static x86_64 Linux binary, no
# glibc dependency) and installs it. Usage:
#
#   curl -fsSL https://raw.githubusercontent.com/mario-valente/pvc-migrator/master/install.sh | bash
#
# Override the install directory with INSTALL_DIR (default: ~/.local/bin —
# no sudo needed; set INSTALL_DIR=/usr/local/bin if you want it system-wide
# and have root).
set -euo pipefail

REPO="mario-valente/pvc-migrator"
BIN_NAME="pvc-migrator"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"
ASSET="pvc-migrator-x86_64-unknown-linux-musl.tar.gz"

arch="$(uname -m)"
os="$(uname -s)"
if [ "$os" != "Linux" ] || [ "$arch" != "x86_64" ]; then
  echo "error: only Linux/x86_64 has a prebuilt binary today (got ${os}/${arch})." >&2
  echo "       build from source instead: cargo install --path ." >&2
  exit 1
fi

tag="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
  | grep '"tag_name"' | head -1 | cut -d'"' -f4)"
if [ -z "$tag" ]; then
  echo "error: couldn't resolve the latest release tag for ${REPO}." >&2
  exit 1
fi

url="https://github.com/${REPO}/releases/download/${tag}/${ASSET}"
echo "Downloading ${BIN_NAME} ${tag} from ${url} ..."

mkdir -p "$INSTALL_DIR"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

curl -fsSL "$url" -o "${tmp}/${ASSET}"
tar -xzf "${tmp}/${ASSET}" -C "$tmp" "$BIN_NAME"
install -m 0755 "${tmp}/${BIN_NAME}" "${INSTALL_DIR}/${BIN_NAME}"

echo "Installed ${INSTALL_DIR}/${BIN_NAME} (${tag})"

case ":${PATH}:" in
  *":${INSTALL_DIR}:"*) ;;
  *)
    echo
    echo "NOTE: ${INSTALL_DIR} is not on your PATH. Add this to your shell profile:"
    echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
    ;;
esac
