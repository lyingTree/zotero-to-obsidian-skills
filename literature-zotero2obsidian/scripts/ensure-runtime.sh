#!/usr/bin/env bash
set -euo pipefail

version="${1:-}"
[[ "$version" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] || { echo "Invalid z2o release tag" >&2; exit 1; }
mode="${2:-runtime}"
case "$mode" in runtime|ocr) ;; *) exit 2 ;; esac
script="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/ensure-runtime.sh"
skill_root="$(dirname -- "$(dirname -- "$script")")"
runtime="$skill_root/runtime"
installed="$runtime/z2o"
case "$(uname -s):$(uname -m)" in
  Linux:x86_64|Linux:amd64) target=x86_64-unknown-linux-gnu ;;
  Linux:aarch64|Linux:arm64) target=aarch64-unknown-linux-gnu ;;
  Darwin:arm64|Darwin:aarch64) target=aarch64-apple-darwin ;;
  *) echo "Unsupported z2o platform" >&2; exit 1 ;;
esac

proxy="${Z2O_PROXY:-${https_proxy:-${HTTPS_PROXY:-${all_proxy:-${ALL_PROXY:-}}}}}"
if [[ -z "$proxy" && "$target" == *-apple-darwin ]]; then
  proxy="$(scutil --proxy | awk '
    $1=="HTTPSEnable" {enabled=$3}
    $1=="HTTPSProxy" {host=$3}
    $1=="HTTPSPort" {port=$3}
    END {if (enabled==1 && host!="" && port ~ /^[0-9]+$/) printf "http://%s:%s",host,port}
  ')" || proxy=''
fi
download() {
  local name="$1" dest="$2" route
  local -a routes=(direct)
  if [[ -n "$proxy" ]]; then
    if [[ "$host" == github && -n "${Z2O_PROXY:-}" ]]; then routes=(proxy direct); else routes=(direct proxy); fi
  fi
  for route in "${routes[@]}"; do
    local -a opts=(--proxy '' --noproxy '*')
    [[ "$route" != proxy ]] || opts=(--proxy "$proxy" --noproxy '')
    if curl --disable "${opts[@]}" -fsSL --connect-timeout 10 --max-time "$3" --speed-limit 1024 --speed-time 25 \
      --retry 1 --retry-max-time 15 --continue-at - --output "$dest" "$url/$name"; then
      return 0
    elif [[ "$?" == 33 ]]; then
      rm -f -- "$dest"
      if curl --disable "${opts[@]}" -fsSL --connect-timeout 10 --max-time "$3" --speed-limit 1024 --speed-time 25 \
        --output "$dest" "$url/$name"; then return 0; fi
    fi
  done
  return 1
}
verified_asset() {
  local name="$1" expected="$2" part="$state/$1.part" digest
  if [[ -f "$part" ]]; then
    digest="$(hash_file "$part")"
    [[ "$digest" != "$expected" ]] || return 0
  fi
  download "$name" "$part" "$3" || return 1
  digest="$(hash_file "$part")"
  if [[ "$digest" != "$expected" ]]; then
    rm -f -- "$part"
    echo "SHA-256 mismatch for $name" >&2
    return 1
  fi
}
hash_file() {
  local value
  if command -v sha256sum >/dev/null 2>&1; then value="$(sha256sum < "$1")"; else value="$(shasum -a 256 < "$1")"; fi
  printf '%s\n' "${value%% *}"
}
runtime_ready() {
  local description
  [[ -x "$1" ]] && description="$("$1" --version 2>/dev/null)" &&
    [[ "$description" =~ \"version\"[[:space:]]*:[[:space:]]*\""$version"\" ]]
}
if [[ -f "$runtime/ocr/package.json" ]]; then
  [[ "$mode" != ocr ]] || exit 0
  if runtime_ready "$installed"; then printf '%s\n' "$installed"; exit 0; fi
fi
region="${Z2O_NETWORK_REGION:-}"
if [[ -z "$region" ]]; then
  trace="$(curl --disable --noproxy '*' -fsS --connect-timeout 2 --max-time 3 https://www.cloudflare.com/cdn-cgi/trace 2>/dev/null)" || trace=''
  region="$(printf '%s\n' "$trace" | awk -F= '$1=="loc" && $2 ~ /^[A-Z][A-Z]$/ {print $2; exit}')"
fi
region="${region:-unknown}"
if [[ "$region" == [Cc][Nn] ]]; then
  host=cnb
  url="https://cnb.cool/lyingTree/zotero-to-obsidian-skills/-/releases/download/$version"
else
  host=github
  url="https://github.com/lyingTree/zotero-to-obsidian-skills/releases/download/$version"
fi
state="$skill_root/user/downloads/$version/$target/$host"
mkdir -p "$state" "$runtime"
lock="$(dirname -- "$state")/$mode.lock"
acquire_lock() {
  local owner
  for owner in "$lock"/[0-9]*; do
    [[ -f "$owner" ]] || continue
    [[ "${owner##*/}" =~ ^[0-9]+$ ]] || continue
    if ! kill -0 "${owner##*/}" 2>/dev/null && rm -- "$owner" 2>/dev/null; then
      rmdir "$lock" 2>/dev/null || true
    fi
  done
  mkdir "$lock" 2>/dev/null || return 1
  : >"$lock/$$"
}
if [[ "$mode" == ocr ]]; then
  [[ ! -f "$runtime/ocr/package.json" ]] || exit 0
  acquire_lock || exit 0
else
  for ((attempt = 0; ; attempt++)); do
    if acquire_lock; then break; fi
    if ((attempt == 120)); then echo "Runtime download already in progress" >&2; exit 1; fi
    sleep 1
  done
fi
stage=''
cleanup() {
  [[ -z "$stage" ]] || rm -rf -- "$stage"
  rm -- "$lock/$$"
  rmdir "$lock"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

sums="$state/SHA256SUMS.txt"
load_sums() {
  [[ -s "$sums" ]] && return 0
  local temp
  temp="$(mktemp "$state/checksums-XXXXXXXX")"
  if ! download SHA256SUMS.txt "$temp" 30; then rm -f -- "$temp"; return 1; fi
  mv -f -- "$temp" "$sums"
}
checksum() {
  awk -v name="$1" '$2 == name || $2 == ("*" name) {print tolower($1); exit}' "$sums"
}

if [[ "$mode" == runtime ]]; then
  if ! runtime_ready "$installed"; then
    load_sums
    name="z2o-$target"
    expected="$(checksum "$name")"
    [[ "$expected" =~ ^[0-9a-f]{64}$ ]] || { echo "Missing checksum for $name" >&2; exit 1; }
    verified_asset "$name" "$expected" 120
    chmod +x "$state/$name.part"
    runtime_ready "$state/$name.part" || { echo "Downloaded z2o cannot start" >&2; exit 1; }
    mv -f -- "$state/$name.part" "$installed"
  fi
  if [[ ! -f "$runtime/ocr/package.json" ]]; then
    Z2O_NETWORK_REGION="$region" nohup bash "$script" "$version" ocr </dev/null >>"$state/ocr.log" 2>&1 &
  fi
  printf '%s\n' "$installed"
else
  [[ ! -f "$runtime/ocr/package.json" ]] || exit 0
  load_sums
  name="z2o-ocr-$target.tar.gz"
  expected="$(checksum "$name")"
  member=ocr
  if [[ -z "$expected" ]]; then
    name="z2o-skills-$target.tar.gz"
    expected="$(checksum "$name")"
    member=skills/literature-zotero2obsidian/runtime/ocr
  fi
  [[ "$expected" =~ ^[0-9a-f]{64}$ ]] || { echo "Missing OCR asset checksum" >&2; exit 1; }
  verified_asset "$name" "$expected" 900
  stage="$(mktemp -d "$state/ocr-XXXXXXXX")"
  tar -xzf "$state/$name.part" -C "$stage" "$member"
  [[ -s "$stage/$member/package.json" && -s "$stage/$member/package.sha256" ]]
  [[ "$(hash_file "$stage/$member/package.json")" == "$(tr -d '[:space:]' < "$stage/$member/package.sha256")" ]]
  if [[ -e "$runtime/ocr" ]]; then echo "OCR destination exists without a manifest: $runtime/ocr" >&2; exit 1; fi
  mv -- "$stage/$member" "$runtime/ocr"
  rm -f -- "$state/$name.part"
  echo "OCR package ready"
fi
