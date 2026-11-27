#!/usr/bin/env bash
set -euo pipefail

version="${1:-}"
if [[ ! "$version" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
  echo "Invalid z2o release tag: $version" >&2
  exit 1
fi

skill_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
runtime="$skill_root/runtime"
installed="$runtime/z2o"

runtime_ready() {
  local description
  [[ -x "$1" ]] && description="$("$1" describe 2>/dev/null)" &&
    [[ "$description" =~ \"engine\"[[:space:]]*:[[:space:]]*\"rust\" ]] &&
    [[ "$description" =~ \"version\"[[:space:]]*:[[:space:]]*\""$version"\" ]]
}

if runtime_ready "$installed"; then
  printf '%s\n' "$installed"
  exit
fi

mkdir -p "$skill_root/user"
lock="$skill_root/user/runtime.lock.d"
for ((attempt = 0; ; attempt++)); do
  if mkdir "$lock" 2>/dev/null; then break; fi
  if ((attempt == 300)); then
    echo "Cannot acquire the z2o runtime installation lock: $lock" >&2
    exit 1
  fi
  sleep 1
done

stage=''
cleanup() {
  if [[ -n "$stage" ]]; then
    if [[ -d "$stage/previous" && ! -e "$runtime" ]]; then
      mv "$stage/previous" "$runtime" || true
    fi
    if [[ ! -d "$stage/previous" ]]; then rm -rf -- "$stage"; fi
  fi
  rmdir "$lock"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

if runtime_ready "$installed"; then
  printf '%s\n' "$installed"
  exit
fi

case "$(uname -s):$(uname -m)" in
  Linux:x86_64|Linux:amd64) target=x86_64-unknown-linux-gnu ;;
  Linux:aarch64|Linux:arm64) target=aarch64-unknown-linux-gnu ;;
  Darwin:arm64|Darwin:aarch64) target=aarch64-apple-darwin ;;
  *) echo "Unsupported z2o platform: $(uname -s) $(uname -m)" >&2; exit 1 ;;
esac

stage="$(mktemp -d "$skill_root/.z2o-runtime-XXXXXXXX")"
name="z2o-skills-$target.tar.gz"
url="https://github.com/lyingTree/zotero-to-obsidian-skills/releases/download/$version"
curl -fsSL "$url/SHA256SUMS.txt" -o "$stage/SHA256SUMS.txt"
expected="$(awk -v name="$name" '$2 == name || $2 == ("*" name) {print tolower($1); exit}' "$stage/SHA256SUMS.txt")"
if [[ ! "$expected" =~ ^[0-9a-f]{64}$ ]]; then
  echo "Release checksum missing for $name" >&2
  exit 1
fi
curl -fsSL "$url/$name" -o "$stage/$name"
if command -v sha256sum >/dev/null 2>&1; then
  digest="$(sha256sum < "$stage/$name")"
else
  digest="$(shasum -a 256 < "$stage/$name")"
fi
if [[ "${digest%% *}" != "$expected" ]]; then
  echo "SHA-256 mismatch for $name" >&2
  exit 1
fi

mkdir "$stage/bundle"
tar -xzf "$stage/$name" -C "$stage/bundle"
staged_runtime="$stage/bundle/skills/literature-zotero2obsidian/runtime"
chmod +x "$staged_runtime/z2o"
if ! runtime_ready "$staged_runtime/z2o"; then
  echo "Release runtime cannot start or does not match $version" >&2
  exit 1
fi

if [[ -e "$runtime" ]]; then mv "$runtime" "$stage/previous"; fi
mv "$staged_runtime" "$runtime"
if ! runtime_ready "$installed"; then
  mv "$runtime" "$stage/failed"
  echo "Installed z2o cannot start" >&2
  exit 1
fi
if [[ -d "$stage/previous" ]]; then mv "$stage/previous" "$stage/replaced"; fi
printf '%s\n' "$installed"
