#!/bin/sh
# Builds Akyba.app (release build, icon, the Claude plugin inside) and signs it.
# `--install` also puts it in /Applications, replacing an older Akyba.app there.
# `--dmg` builds it twice, for Apple Silicon and for Intel, each in a disk image of its own:
# one app for both would carry both programs, twice the download for everyone.
set -eu
cd "$(dirname "$0")/.."
mode=${1:-}

# A `.noindex` folder, so Spotlight doesn't list the build next to the installed app.
out=target/bundle.noindex
rm -rf target/release/Akyba.app target/release/dmg
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)

# Akyba.app for one kind of Mac (`apple-silicon` or `intel`) from its program, in `$app`. Each
# kind reads its own update feed, so Sparkle only ever offers it the build made for it.
bundle() {
  kind=$1
  binary=$2
  app=$out/$kind/Akyba.app
  rm -rf "$app"
  mkdir -p "$out" "$app/Contents/MacOS" "$app/Contents/Resources"
  cp "$binary" "$app/Contents/MacOS/akyba"
  cp crates/akyba/assets/app/Akyba.icns "$app/Contents/Resources/Akyba.icns"
  # Sparkle, which Akyba loads at launch to update itself from the GitHub releases.
  sparkle=$(sh scripts/sparkle.sh)
  mkdir -p "$app/Contents/Frameworks"
  cp -R "$sparkle/Sparkle.framework" "$app/Contents/Frameworks/"
  cp -R plugin "$app/Contents/Resources/plugin"

  cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Akyba</string>
  <key>CFBundleDisplayName</key><string>Akyba</string>
  <key>CFBundleIdentifier</key><string>dev.akyba.workbench</string>
  <key>CFBundleExecutable</key><string>akyba</string>
  <key>CFBundleIconFile</key><string>Akyba</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
  <key>NSHighResolutionCapable</key><true/>
  <!-- Updates: the newest GitHub release's feed, signed with the key in the publisher's keychain;
       checked once a day, so an update put off with Remind me later comes back the next day. -->
  <key>SUFeedURL</key><string>https://github.com/itsjonasarden/akyba/releases/latest/download/appcast-$kind.xml</string>
  <key>SUPublicEDKey</key><string>L6qK9LpfH0C48mSjQNc4JAnNxO7rokEYbdIR5gemae0=</string>
  <key>SUEnableAutomaticChecks</key><true/>
  <key>SUScheduledCheckInterval</key><integer>86400</integer>
</dict>
</plist>
PLIST

  # Without an Apple Developer ID it isn't notarised, so a Mac that downloaded it asks once
  # (System Settings → Privacy & Security → Open Anyway).
  #
  # Signed with the publisher's own "Akyba Code Signing" certificate when it's in the keychain
  # (self-signed: it needn't be trusted, so it's named by its fingerprint). Every version then
  # carries the same identity, and macOS keeps "Always Allow" for Akyba's Keychain items across
  # updates; an ad-hoc signature changes with every build, so each update asked again. Anyone
  # building without it gets the ad-hoc signature.
  identity=$(security find-identity -p codesigning 2>/dev/null | awk '/"Akyba Code Signing"/ { print $2; exit }')
  codesign --force --deep --sign "${identity:--}" "$app"
  if [ -n "$identity" ]; then echo "Signed with Akyba Code Signing ($identity)"; fi
  echo "Built $app ($version, $kind)"
}

# `$app` packed into target/Akyba-<version>-<kind>.dmg, compressed with LZMA (ULMO), which every
# macOS Akyba runs on opens and which packs tighter than the default.
disk_image() {
  kind=$1
  stage=$out/dmg
  rm -rf "$stage"
  mkdir -p "$stage"
  cp -R "$app" "$stage/Akyba.app"
  cp scripts/how-to-open.txt "$stage/How to open Akyba.txt"
  dmg="target/Akyba-$version-$kind.dmg"
  rm -f "$dmg"
  if command -v uvx >/dev/null 2>&1; then
    # Laid out with a background and Akyba's icon on the disk (scripts/dmg.py).
    uvx --quiet --from dmgbuild==1.6.7 dmgbuild -s scripts/dmg.py -D stage="$stage" "Akyba $version" "$dmg"
  else
    # Without uv, a plain image: the same files, in Finder's default window.
    ln -s /Applications "$stage/Applications"
    hdiutil create -quiet -volname "Akyba $version" -srcfolder "$stage" -format ULMO "$dmg"
  fi
  rm -rf "$stage"
  echo "Built $dmg"
}

if [ "$mode" = "--dmg" ]; then
  for kind in apple-silicon intel; do
    case $kind in apple-silicon) target=aarch64-apple-darwin ;; intel) target=x86_64-apple-darwin ;; esac
    cargo build --release -p akyba --target "$target"
    bundle "$kind" "target/$target/release/akyba"
    disk_image "$kind"
  done
else
  cargo build --release -p akyba
  case $(uname -m) in arm64) kind=apple-silicon ;; *) kind=intel ;; esac
  bundle "$kind" target/release/akyba
fi

if [ "$mode" = "--install" ]; then
  rm -rf /Applications/Akyba.app
  cp -R "$app" /Applications/Akyba.app
  echo "Installed /Applications/Akyba.app"
fi
