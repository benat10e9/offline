#!/usr/bin/env bash
# Build a customer's drive from their signed profile (profile.json + profile.sig).
#
#   scripts/build-drive.sh <profile-dir> <drive-or-folder> [--kb path/to/kb]
#
# Installs, for every OS in the profile: llama.cpp runtimes (pinned, SHA-256 checked) and the
# desktop app; then the profile's model and packages, the profile itself and the trust key.
# Ends with `kb verify-drive`, which must pass. Re-running repairs a drive: good files are
# kept, missing or damaged ones are fetched again.
set -euo pipefail

REPO="${KB_REPO:-benat10e9/offline}"
LLAMA_TAG="b11747"
PROFILE_DIR="${1:?profile dir}"
ROOT="${2:?drive or folder}"
KB="${KB_BIN:-}"
shift 2
while [ $# -gt 0 ]; do case "$1" in --kb) KB="$2"; shift 2 ;; *) echo "unknown arg $1"; exit 1 ;; esac; done
[ -n "$KB" ] || KB="$(command -v kb || true)"
[ -x "$KB" ] || { echo "need the kb tool (pass --kb path/to/kb)"; exit 1; }

say() { printf '\n== %s\n' "$*"; }
sha256() { if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }
fetch() { curl -fL --retry 4 --retry-delay 3 -C - --progress-bar -o "$2" "$1"; }
release_asset() { # tag name dest
  if command -v gh >/dev/null && gh auth status >/dev/null 2>&1; then
    gh release download "$1" --repo "$REPO" --pattern "$2" --output "$3" --clobber
  else
    fetch "https://github.com/$REPO/releases/download/$1/$2" "$3"
  fi
}
field() { python3 -c "import json,sys;d=json.load(open('$PROFILE_DIR/profile.json'));v=d$1;print(' '.join(v) if isinstance(v,list) else v)"; }

[ -f "$PROFILE_DIR/profile.json" ] && [ -f "$PROFILE_DIR/profile.sig" ] || { echo "profile.json and profile.sig required"; exit 1; }
mkdir -p "$ROOT"/{bin,models,packages,trust,private,updates,LICENSES}
ROOT="$(cd "$ROOT" && pwd)"
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT

OS_LIST="$(field "['os']")"
MODEL="$(field "['model']")"
PACKAGES="$(field "['packages']")"
say "Profile $(field "['profile_id']") v$(field "['version']"): os=[$OS_LIST] model=$MODEL packages=[$PACKAGES]"

cp "$PROFILE_DIR/profile.json" "$PROFILE_DIR/profile.sig" "$ROOT/"
release_asset content-dev "dev-publisher.pub" "$ROOT/trust/dev-publisher.pub"
# The store signs profiles; in development it uses the same key. Add its key if different.
[ -n "${STORE_PUBKEY:-}" ] && printf "%s\n" "$STORE_PUBKEY" > "$ROOT/trust/store.pub"

install_llama() { # platform-dir backend asset sha
  local dest="$ROOT/bin/$1/$2"
  if [ -f "$dest/llama-server" ] || [ -f "$dest/llama-server.exe" ]; then echo "  $1/$2: present"; return; fi
  fetch "https://github.com/ggml-org/llama.cpp/releases/download/$LLAMA_TAG/$3" "$TMP/$3"
  [ "$(sha256 "$TMP/$3")" = "$4" ] || { echo "CHECKSUM MISMATCH $3"; exit 2; }
  mkdir -p "$TMP/x-$1-$2" "$dest"
  case "$3" in *.zip) unzip -q "$TMP/$3" -d "$TMP/x-$1-$2" ;; *) tar xzf "$TMP/$3" -C "$TMP/x-$1-$2" ;; esac
  local server; server="$(find "$TMP/x-$1-$2" \( -name llama-server -o -name llama-server.exe \) -type f | head -1)"
  cp -R "$(dirname "$server")"/. "$dest"/
  echo "  $1/$2: installed"
}
install_app() { # asset platform-dir
  release_asset app-dev "$1" "$TMP/$1"
  rm -rf "$TMP/app" && mkdir -p "$TMP/app" && unzip -oq "$TMP/$1" -d "$TMP/app"
  find "$TMP/app" -maxdepth 1 -name "*.app" -exec cp -R {} "$ROOT/" \;
  [ -f "$TMP/app/Offline Library.exe" ] && cp "$TMP/app/Offline Library.exe" "$ROOT/"
  mkdir -p "$ROOT/bin/$2"
  for k in kb kb.exe; do [ -f "$TMP/app/$k" ] && cp "$TMP/app/$k" "$ROOT/bin/$2/"; done
  return 0
}

say "Runtimes and apps"
for os in $OS_LIST; do
  case "$os" in
    macos)
      install_llama macos-aarch64 metal "llama-$LLAMA_TAG-bin-macos-arm64.tar.gz" a2ac5500723549481b561ea9461447387010c5bb827ecb14fb29e464dda78ee5
      install_app offline-library-macos-arm64.zip macos-aarch64 ;;
    windows)
      install_llama windows-x86_64 vulkan "llama-$LLAMA_TAG-bin-win-vulkan-x64.zip" d298dc743c1e6b73b441742efedc40667232b77c0da5484aaf54475470d76198
      install_llama windows-x86_64 cpu "llama-$LLAMA_TAG-bin-win-cpu-x64.zip" 5d9f9ab6406bcc6560d60cdaf94464fc3ae661ef2e4b35cdedad3a048f1cf0bf
      install_app offline-library-windows-x86_64.zip windows-x86_64 ;;
    linux)
      install_llama linux-x86_64 cpu "llama-$LLAMA_TAG-bin-ubuntu-x64.tar.gz" 6e9e6eb4db7e2133fed8dfaf141fb1abfdd5b18db4b360be45245e0f91ffca1e
      install_app offline-library-linux-x86_64.zip linux-x86_64 ;;
    *) echo "unknown os $os"; exit 1 ;;
  esac
done

say "Model $MODEL"
case "$MODEL" in
  qwen3-4b-q4km) FILE=Qwen3-4B-Q4_K_M.gguf; URL=https://huggingface.co/Qwen/Qwen3-4B-GGUF/resolve/main/Qwen3-4B-Q4_K_M.gguf; RAM=12 ;;
  qwen3-8b-q4km) FILE=Qwen3-8B-Q4_K_M.gguf; URL=https://huggingface.co/Qwen/Qwen3-8B-GGUF/resolve/main/Qwen3-8B-Q4_K_M.gguf; RAM=16 ;;
  *) echo "unknown model $MODEL"; exit 1 ;;
esac
if [ -f "$ROOT/models/$FILE" ] && [ -f "$ROOT/models/models.json" ] && grep -q "$(sha256 "$ROOT/models/$FILE")" "$ROOT/models/models.json"; then
  echo "  present and verified"
else
  fetch "${KB_MODEL_MIRROR:-$URL}" "$ROOT/models/$FILE.part" && mv "$ROOT/models/$FILE.part" "$ROOT/models/$FILE"
  printf '{"models":[{"id":"%s","file":"%s","min_ram_gb":%s,"context":8192,"license":"Apache-2.0","safety_approved":false,"sha256":"%s"}]}\n' \
    "$MODEL" "$FILE" "$RAM" "$(sha256 "$ROOT/models/$FILE")" > "$ROOT/models/models.json"
fi
# Remove models the profile no longer selects (configuration changed).
find "$ROOT/models" -name "*.gguf" ! -name "$FILE" -delete

say "Packages"
for pkg in $PACKAGES; do
  if [ -d "$ROOT/packages/$pkg" ] && "$KB" verify --full --pkg "$ROOT/packages/$pkg" --trust "$ROOT/trust/dev-publisher.pub" >/dev/null 2>&1; then
    echo "  $pkg: present and verified"; continue
  fi
  rm -rf "${ROOT:?}/packages/$pkg"
  case "$pkg" in
    maps-*) release_asset maps-dev "$pkg.tar.gz" "$TMP/$pkg.tgz" ;;
    *)
      release_asset content-dev "$pkg.parts.txt" "$TMP/$pkg.parts"
      : > "$TMP/$pkg.tgz"
      while read -r part; do [ -n "$part" ] && release_asset content-dev "$part" "$TMP/part" && cat "$TMP/part" >> "$TMP/$pkg.tgz"; done < "$TMP/$pkg.parts" ;;
  esac
  tar xzf "$TMP/$pkg.tgz" -C "$ROOT/packages" && rm -f "$TMP/$pkg.tgz" "$TMP/part"
  echo "  $pkg: installed"
done
# Remove packages the profile no longer selects.
for d in "$ROOT"/packages/*/; do
  [ -d "$d" ] || continue
  name="$(basename "$d")"
  case " $PACKAGES " in *" $name "*) ;; *) echo "  removing $name (no longer in profile)"; rm -rf "$d" ;; esac
done

say "Verify"
"$KB" verify-drive --root "$ROOT"
du -sh "$ROOT"
echo "Drive ready."
