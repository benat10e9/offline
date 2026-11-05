#!/usr/bin/env bash
# Provision a library drive (or any folder) on macOS or Linux.
#
#   scripts/provision.sh /Volumes/LIBRARY            # 4B and 8B models (default)
#   scripts/provision.sh ~/library --models 4b       # one model
#   scripts/provision.sh ~/library --no-models       # library + app only
#
# Downloads, verifies and lays out (see spec/ssd-layout.md):
#   bin/<platform>/<backend>/   llama.cpp build b11747, checksum-pinned
#   bin/<platform>/kb           command-line tool (bench, serve) from the app-dev release
#   models/                     GGUF models + models.json
#   packages/                   signed content packages from the content-dev release
#   trust/                      development publisher key
#   Offline Library.app         desktop app (macOS) from the app-dev release
# and marks the drive as a development drive (unevaluated models allowed, with a warning).
set -euo pipefail

REPO="${KB_REPO:-benat10e9/offline}"
LLAMA_TAG="b11747"
ROOT=""
MODELS="both"

while [ $# -gt 0 ]; do
  case "$1" in
    --models) MODELS="$2"; shift 2 ;;
    --no-models) MODELS="none"; shift ;;
    -h|--help) sed -n '2,20p' "$0"; exit 0 ;;
    *) ROOT="$1"; shift ;;
  esac
done
[ -n "$ROOT" ] || { echo "usage: $0 <drive-or-folder> [--models 4b|8b|both] [--no-models]"; exit 1; }
mkdir -p "$ROOT"/{bin,models,packages,trust,private,updates,LICENSES}
ROOT="$(cd "$ROOT" && pwd)"

say() { printf '\n== %s\n' "$*"; }
sha256() { if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }
fetch() { # url dest
  curl -fL --retry 4 --retry-delay 3 -C - --progress-bar -o "$2" "$1"
}
release_asset() { # tag name dest   (uses gh if available, which also works for private repos)
  if command -v gh >/dev/null && gh auth status >/dev/null 2>&1; then
    gh release download "$1" --repo "$REPO" --pattern "$2" --output "$3" --clobber
  else
    fetch "https://github.com/$REPO/releases/download/$1/$2" "$3"
  fi
}

OS="$(uname -s)"; ARCH="$(uname -m)"
case "$OS-$ARCH" in
  Darwin-arm64)  PLAT="macos-aarch64"; BUILDS=("metal:llama-$LLAMA_TAG-bin-macos-arm64.tar.gz:a2ac5500723549481b561ea9461447387010c5bb827ecb14fb29e464dda78ee5") ;;
  Darwin-x86_64) PLAT="macos-x86_64";  BUILDS=("cpu:llama-$LLAMA_TAG-bin-macos-x64.tar.gz:116981b22c4aebd14b74cbfec90dae8860731d46a3f6ef8678ea629372b68375") ;;
  Linux-x86_64)  PLAT="linux-x86_64";  BUILDS=("vulkan:llama-$LLAMA_TAG-bin-ubuntu-vulkan-x64.tar.gz:28ee91a2056856698678801ce63b9d8b87ede0428f96f3609b38c1f7fa8b1f82"
                                              "cpu:llama-$LLAMA_TAG-bin-ubuntu-x64.tar.gz:6e9e6eb4db7e2133fed8dfaf141fb1abfdd5b18db4b360be45245e0f91ffca1e") ;;
  *) echo "Unsupported platform $OS-$ARCH (Windows: use scripts/provision.ps1)"; exit 1 ;;
esac
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT

say "AI engine: llama.cpp $LLAMA_TAG for $PLAT"
for b in "${BUILDS[@]}"; do
  IFS=: read -r backend asset want <<<"$b"
  dest="$ROOT/bin/$PLAT/$backend"
  if [ -x "$dest/llama-server" ]; then echo "  $backend: already installed"; continue; fi
  fetch "https://github.com/ggml-org/llama.cpp/releases/download/$LLAMA_TAG/$asset" "$TMP/$asset"
  got="$(sha256 "$TMP/$asset")"
  [ "$got" = "$want" ] || { echo "CHECKSUM MISMATCH for $asset ($got)"; exit 2; }
  mkdir -p "$TMP/x-$backend" "$dest"
  tar xzf "$TMP/$asset" -C "$TMP/x-$backend"
  src="$(dirname "$(find "$TMP/x-$backend" -name llama-server -type f | head -1)")"
  cp -R "$src"/. "$dest"/
  echo "  $backend: ok ($(du -sh "$dest" | cut -f1))"
done
cp -n "$(find "$ROOT/bin/$PLAT" -name LICENSE | head -1)" "$ROOT/LICENSES/llama.cpp-LICENSE.txt" 2>/dev/null || true

# Models: Apache-2.0 Qwen3 builds, 4-bit. min_ram_gb is when the app offers each by default;
# safety_approved stays false until the safety evaluation passes.
declare -a MODEL_LINES=()
add_model() { # id file url min_ram
  local id="$1" file="$2" url="$3" ram="$4"
  if [ ! -f "$ROOT/models/$file" ]; then
    say "Model: $id (this is the big download)"
    fetch "$url" "$ROOT/models/$file.part" && mv "$ROOT/models/$file.part" "$ROOT/models/$file"
  else
    say "Model: $id already present"
  fi
  local sum; sum="$(sha256 "$ROOT/models/$file")"
  MODEL_LINES+=("{\"id\":\"$id\",\"file\":\"$file\",\"min_ram_gb\":$ram,\"context\":8192,\"license\":\"Apache-2.0\",\"safety_approved\":false,\"sha256\":\"$sum\"}")
}
HF="https://huggingface.co"
case "$MODELS" in
  4b|both) add_model "qwen3-4b-q4km" "Qwen3-4B-Q4_K_M.gguf" "$HF/Qwen/Qwen3-4B-GGUF/resolve/main/Qwen3-4B-Q4_K_M.gguf" 12 ;;
esac
case "$MODELS" in
  8b|both) add_model "qwen3-8b-q4km" "Qwen3-8B-Q4_K_M.gguf" "$HF/Qwen/Qwen3-8B-GGUF/resolve/main/Qwen3-8B-Q4_K_M.gguf" 16 ;;
esac
if [ ${#MODEL_LINES[@]} -gt 0 ]; then
  (IFS=,; printf '{"models":[%s]}\n' "${MODEL_LINES[*]}") > "$ROOT/models/models.json"
fi

say "Content packages (content-dev release)"
if release_asset content-dev "packages.tar.gz" "$TMP/packages.tar.gz" 2>/dev/null; then
  tar xzf "$TMP/packages.tar.gz" -C "$ROOT/packages"
  ls "$ROOT/packages"
else
  echo "  not published yet; run the 'content' workflow in GitHub Actions, then re-run this script"
fi
release_asset content-dev "dev-publisher.pub" "$ROOT/trust/dev-publisher.pub" 2>/dev/null \
  || echo "  publisher key not published yet"

say "App (app-dev release)"
case "$PLAT" in
  macos-*)  APP_ASSET="offline-library-macos-$ARCH.zip" ;;
  linux-*)  APP_ASSET="offline-library-linux-x86_64.zip" ;;
esac
if release_asset app-dev "$APP_ASSET" "$TMP/app.zip" 2>/dev/null; then
  unzip -oq "$TMP/app.zip" -d "$TMP/app"
  find "$TMP/app" -maxdepth 2 -name "*.app" -exec cp -R {} "$ROOT/" \; 2>/dev/null || true
  [ -f "$TMP/app/kb" ] && { mkdir -p "$ROOT/bin/$PLAT"; cp "$TMP/app/kb" "$ROOT/bin/$PLAT/kb"; chmod +x "$ROOT/bin/$PLAT/kb"; }
  find "$TMP/app" -maxdepth 2 -name "*.AppImage" -exec cp {} "$ROOT/" \; 2>/dev/null || true
  echo "  ok"
else
  echo "  not published yet; run the 'app' workflow in GitHub Actions, then re-run this script"
fi

touch "$ROOT/DEV_ALLOW_UNEVALUATED"

say "Done: $ROOT"
du -sh "$ROOT"/* 2>/dev/null | sort -h
cat <<EOF

Next:
  Open the app:   open "$ROOT/Offline Library.app"      (macOS)
  Browser mode:   "$ROOT/bin/$PLAT/kb" serve --root "$ROOT" --allow-unevaluated
  Benchmark:      "$ROOT/bin/$PLAT/kb" bench --root "$ROOT" --allow-unevaluated
                  then send bench-report.json back.
EOF
