#!/bin/sh
# Fetches Sparkle (the macOS updater Akyba ships with) once, into target/sparkle-<version>, and
# checks it against the pinned checksum. Prints the folder.
set -eu
cd "$(dirname "$0")/.."

version=2.10.0
sha256=c2bf58aa8387266ac179357b1415d6f2635f044da8be41042af32425dae6da0c
dir=target/sparkle-$version

if [ ! -d "$dir/Sparkle.framework" ]; then
  mkdir -p "$dir"
  archive="$dir/Sparkle-$version.tar.xz"
  curl -sSL -o "$archive" "https://github.com/sparkle-project/Sparkle/releases/download/$version/Sparkle-$version.tar.xz"
  echo "$sha256  $archive" | shasum -a 256 -c - >/dev/null
  tar -xf "$archive" -C "$dir"
  rm "$archive"
fi
echo "$dir"
