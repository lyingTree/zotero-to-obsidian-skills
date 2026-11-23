#!/usr/bin/env bash
# Install external dependency skills: obsidian-cli, zotero-skills.
# Fetches them from their upstream repos and installs into the Skills root
# (same level as literature-*).
#
# Usage:
#   ./install.sh                       # install next to this script
#   ./install.sh <dir>                 # install into a given Skills root
#   ./install.sh <dir> <obs-ref> <zotero-ref>   # pin versions (default branch if empty)
set -euo pipefail

SKILLS_ROOT="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)}"
OBS_REF="${2:-}"
ZOT_REF="${3:-}"

OBSIDIAN_REPO="https://github.com/obsidianmd/obsidian-skills.git"
ZOTERO_REPO="https://github.com/WenyuChiou/zotero-skills.git"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

clone_repo() { # url dest ref
  local url="$1" dest="$2" ref="$3"
  if [ -n "$ref" ]; then
    git clone --depth 1 --branch "$ref" "$url" "$dest"
  else
    git clone --depth 1 "$url" "$dest"
  fi
}

echo "==> Fetching obsidianmd/obsidian-skills ..."
clone_repo "$OBSIDIAN_REPO" "$tmp/obsidian-skills" "$OBS_REF"
echo "==> Fetching WenyuChiou/zotero-skills ..."
clone_repo "$ZOTERO_REPO" "$tmp/zotero-skills" "$ZOT_REF"

echo "==> Installing obsidian-cli -> $SKILLS_ROOT/obsidian-cli"
rm -rf "$SKILLS_ROOT/obsidian-cli"
cp -R "$tmp/obsidian-skills/plugins/obsidian/skills/obsidian-cli" "$SKILLS_ROOT/obsidian-cli"

echo "==> Installing zotero-skills -> $SKILLS_ROOT/zotero-skills"
rm -rf "$SKILLS_ROOT/zotero-skills"
cp -R "$tmp/zotero-skills/skills/zotero-skills" "$SKILLS_ROOT/zotero-skills"
cp -R "$tmp/zotero-skills/scripts" "$SKILLS_ROOT/zotero-skills/scripts"
cp "$tmp/zotero-skills/requirements.txt" "$SKILLS_ROOT/zotero-skills/requirements.txt"
cp "$tmp/zotero-skills/LICENSE" "$SKILLS_ROOT/zotero-skills/LICENSE"

echo "==> Installing Python deps (requirements.txt) ..."
PY="$(command -v python3 || command -v python || true)"
if [ -z "$PY" ]; then
  echo "Python not found on PATH. Install Python 3.10+ then run: python -m pip install -r \"$SKILLS_ROOT/zotero-skills/requirements.txt\"" >&2
elif ! "$PY" -m pip install -r "$SKILLS_ROOT/zotero-skills/requirements.txt"; then
  echo "pip install failed. Run manually: \"$PY\" -m pip install -r \"$SKILLS_ROOT/zotero-skills/requirements.txt\"" >&2
fi

echo "Done. obsidian-cli requires the Obsidian desktop app installed and running."
