#!/usr/bin/env bash
# Install the 12 native literature-* skills (with precompiled runtime/z2o)
# from GitHub Releases, plus optional external dependency skills (obsidian-cli, zotero-skills).
# No Python or local Rust compilation required.
#
# Usage: ./install.sh [skills-root] [version-tag] [obsidian-ref] [zotero-ref]
set -euo pipefail

if [ "$#" -gt 4 ]; then
  echo "Usage: ./install.sh [skills-root] [version-tag] [obsidian-ref] [zotero-ref]" >&2
  exit 2
fi

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
SKILLS_ROOT="${1:-${Z2O_SKILLS_ROOT:-$SCRIPT_DIR}}"
VERSION="${2:-${Z2O_VERSION:-}}"
OBSIDIAN_REF="${3:-${Z2O_OBSIDIAN_SKILLS_REF:-b110b2f8e9eb1f287892f937050dfd40af862cbb}}"
ZOTERO_REF="${4:-${Z2O_ZOTERO_SKILLS_REF:-e5c11e744fab34d9dd322319195249daf735c2b1}}"
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
if [ -n "$VERSION" ] && [[ "$VERSION" != v* ]]; then
  VERSION="v$VERSION"
fi
if [ -z "$VERSION" ]; then
  echo "Version tag is required (or place VERSION file next to install.sh)." >&2
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
echo "==> Installing 12 native literature-* skills into ${SKILLS_ROOT} ..."
"$Z2O_BIN" install "$MANIFEST" "$MANIFEST_SHA" "$SKILLS_ROOT"
"${SKILLS_ROOT}/literature-zotero2obsidian/runtime/z2o" verify-install "$MANIFEST" "$MANIFEST_SHA" "$SKILLS_ROOT" >/dev/null

if [ "${Z2O_SKIP_EXTERNAL_DEPENDENCIES:-0}" != "1" ] && command -v git >/dev/null 2>&1; then
  echo "==> Fetching pinned external skills (obsidian-cli@${OBSIDIAN_REF}, zotero-skills@${ZOTERO_REF}) ..."
  OBS_TMP="${TMP_DIR}/obsidian-skills"
  git init -q "$OBS_TMP"
  git -C "$OBS_TMP" remote add origin "https://github.com/obsidianmd/obsidian-skills.git"
  git -C "$OBS_TMP" fetch -q --depth 1 origin "$OBSIDIAN_REF"
  git -C "$OBS_TMP" checkout -q FETCH_HEAD
  rm -rf "${SKILLS_ROOT}/obsidian-cli"
  cp -R "${OBS_TMP}/plugins/obsidian/skills/obsidian-cli" "${SKILLS_ROOT}/obsidian-cli"
  cp "${OBS_TMP}/LICENSE" "${SKILLS_ROOT}/obsidian-cli/LICENSE"

  ZOT_TMP="${TMP_DIR}/zotero-skills"
  git init -q "$ZOT_TMP"
  git -C "$ZOT_TMP" remote add origin "https://github.com/WenyuChiou/zotero-skills.git"
  git -C "$ZOT_TMP" fetch -q --depth 1 origin "$ZOTERO_REF"
  git -C "$ZOT_TMP" checkout -q FETCH_HEAD
  rm -rf "${SKILLS_ROOT}/zotero-skills"
  cp -R "${ZOT_TMP}/skills/zotero-skills" "${SKILLS_ROOT}/zotero-skills"
  cp -R "${ZOT_TMP}/scripts" "${SKILLS_ROOT}/zotero-skills/scripts"
  cp "${ZOT_TMP}/requirements.txt" "${SKILLS_ROOT}/zotero-skills/requirements.txt"
  cp "${ZOT_TMP}/LICENSE" "${SKILLS_ROOT}/zotero-skills/LICENSE"
fi

echo "==> Native installation complete (${VERSION}, ${TARGET})."
