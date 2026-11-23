#!/usr/bin/env bash
# Compatibility wrapper. For script-based use, edit and run install.py.
# Usage: ./install.sh [skills-root] [obsidian-ref] [zotero-ref]
set -euo pipefail

if [ "$#" -gt 3 ]; then
  echo "Usage: ./install.sh [skills-root] [obsidian-ref] [zotero-ref]" >&2
  exit 2
fi

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
if [ "$#" -ge 1 ]; then export Z2O_SKILLS_ROOT="$1"; fi
if [ "$#" -ge 2 ]; then export Z2O_OBSIDIAN_SKILLS_REF="$2"; fi
if [ "$#" -ge 3 ]; then export Z2O_ZOTERO_SKILLS_REF="$3"; fi

if [ -n "${Z2O_PYTHON:-}" ]; then
  exec "$Z2O_PYTHON" "$SCRIPT_DIR/install.py"
fi
if command -v python3 >/dev/null 2>&1; then
  exec python3 "$SCRIPT_DIR/install.py"
fi
if command -v python >/dev/null 2>&1; then
  exec python "$SCRIPT_DIR/install.py"
fi
echo "Python 3.10+ is required. Set Z2O_PYTHON to the interpreter to use." >&2
exit 1
