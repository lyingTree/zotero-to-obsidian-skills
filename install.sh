#!/usr/bin/env bash
# All-in-one installer for the 12 literature-* skills, precompiled Rust runtime/z2o,
# and bundled PP-OCRv6 OCR/layout/table runtime (when included in the release bundle)
# from GitHub Releases.
# No local compilation and no first-run OCR download prompts required.
#
# Usage: ./install.sh [skills-root] [version-tag]
set -euo pipefail

# Accept the two legacy trailing arguments for existing installer callers.
if [ "$#" -gt 4 ]; then
  echo "Usage: ./install.sh [skills-root] [version-tag]" >&2
  exit 2
fi

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
SKILLS_ROOT="${1:-${SKILLS_ROOT:-${Z2O_SKILLS_ROOT:-}}}"
if [ -z "$SKILLS_ROOT" ]; then
  if [ -f "$SCRIPT_DIR/rust/Cargo.toml" ]; then
    echo "Refusing to install into the source repository root ($SCRIPT_DIR). Pass [skills-root] or set SKILLS_ROOT=/path/to/skills." >&2
    exit 1
  fi
  SKILLS_ROOT="$SCRIPT_DIR"
fi

VERSION="${2:-${Z2O_VERSION:-}}"
REPO="${Z2O_REPO:-lyingTree/zotero-to-obsidian-skills}"

OS="$(uname -s)"
ARCH="$(uname -m)"
case "$OS:$ARCH" in
  Linux:x86_64|Linux:amd64)
    TARGET="x86_64-unknown-linux-gnu"
    ;;
  Linux:aarch64|Linux:arm64)
    TARGET="aarch64-unknown-linux-gnu"
    ;;
  Darwin:arm64|Darwin:aarch64)
    TARGET="aarch64-apple-darwin"
    ;;
  *)
    echo "Unsupported OS/architecture: $OS / $ARCH" >&2
    exit 1
    ;;
esac

if [ -z "$VERSION" ] && [ -f "$SCRIPT_DIR/VERSION" ]; then
  VERSION="$(tr -d '[:space:]' < "$SCRIPT_DIR/VERSION")"
fi
if [ -z "$VERSION" ]; then
  VERSION="$(curl -fsSL -H "User-Agent: z2o-installer" "https://api.github.com/repos/${REPO}/releases/latest" \
    | grep -o '"tag_name"[[:space:]]*:[[:space:]]*"[^"]*"' | head -n 1 \
    | sed 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)"/\1/')"
fi
if [ -n "$VERSION" ] && [[ "$VERSION" != v* ]]; then
  VERSION="v$VERSION"
fi
if [ -z "$VERSION" ]; then
  echo "Could not determine release version tag from VERSION file or GitHub releases." >&2
  exit 1
fi

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum < "$1" | awk '{print $1}' | tr -d '\r'
  else
    shasum -a 256 < "$1" | awk '{print $1}' | tr -d '\r'
  fi
}

ARCHIVE="z2o-skills-${TARGET}.tar.gz"
BASE_URL="https://github.com/${REPO}/releases/download/${VERSION}"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

echo "==> Downloading ${ARCHIVE} and SHA256SUMS.txt from ${REPO}@${VERSION} ..."
curl -fsSL "${BASE_URL}/${ARCHIVE}" -o "${TMP_DIR}/${ARCHIVE}"
curl -fsSL "${BASE_URL}/SHA256SUMS.txt" -o "${TMP_DIR}/SHA256SUMS.txt"

EXPECTED_SHA="$(awk -v name="$ARCHIVE" '$2 == name || $2 == ("*" name) {print tolower($1)}' "${TMP_DIR}/SHA256SUMS.txt" | head -n 1)"
if [ -z "$EXPECTED_SHA" ]; then
  echo "Archive ${ARCHIVE} not found in SHA256SUMS.txt" >&2
  exit 1
fi
ACTUAL_SHA="$(sha256_file "${TMP_DIR}/${ARCHIVE}")"
if [ "$ACTUAL_SHA" != "$EXPECTED_SHA" ]; then
  echo "SHA-256 mismatch for ${ARCHIVE}: expected ${EXPECTED_SHA}, got ${ACTUAL_SHA}" >&2
  exit 1
fi

BUNDLE_DIR="${TMP_DIR}/bundle"
mkdir -p "$BUNDLE_DIR"
tar -xzf "${TMP_DIR}/${ARCHIVE}" -C "$BUNDLE_DIR"

MANIFEST="${BUNDLE_DIR}/package.json"
Z2O_BIN="${BUNDLE_DIR}/skills/literature-zotero2obsidian/runtime/z2o"
chmod +x "$Z2O_BIN"
MANIFEST_SHA="$(sha256_file "$MANIFEST")"

mkdir -p "$SKILLS_ROOT"
echo "==> Installing 12 literature-* skills, runtime/z2o, and any bundled OCR runtime into ${SKILLS_ROOT} ..."
"$Z2O_BIN" install "$MANIFEST" "$MANIFEST_SHA" "$SKILLS_ROOT"
"${SKILLS_ROOT}/literature-zotero2obsidian/runtime/z2o" verify-install "$MANIFEST" "$MANIFEST_SHA" "$SKILLS_ROOT"

echo "==> Rust skill installation complete (${VERSION}, ${TARGET})."
