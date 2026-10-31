#!/bin/sh
# Publishes a version of Akyba: `pnpm release 0.2.0`.
#
# Sets the version, commits and tags it, builds a disk image for Apple Silicon and one for Intel,
# signs each with the Sparkle key in this Mac's keychain, and publishes a GitHub release carrying
# them and a feed for each kind of Mac (`appcast-apple-silicon.xml`, `appcast-intel.xml`), which
# every installed Akyba of that kind checks once a day. The newest release's feeds are the ones
# they read (releases/latest/download/appcast-<kind>.xml).
#
# The signing key was made with Sparkle's `generate_keys`; back it up with
# `target/sparkle-2.10.0/bin/generate_keys -x <file>`. Without it, installed copies can't be updated.
set -eu
cd "$(dirname "$0")/.."

version=${1:-}
case "$version" in
  [0-9]*.[0-9]*.[0-9]*) ;;
  *) echo "Usage: pnpm release <major.minor.patch>, e.g. pnpm release 0.2.0" >&2; exit 1 ;;
esac
tag="v$version"

# Only what's committed and pushed on main goes out.
[ "$(git branch --show-current)" = main ] || { echo "Release from main." >&2; exit 1; }
[ -z "$(git status --porcelain)" ] || { echo "Commit or stash your changes first." >&2; exit 1; }
git fetch -q origin main
[ "$(git rev-parse HEAD)" = "$(git rev-parse origin/main)" ] || { echo "Push main first." >&2; exit 1; }
git rev-parse -q --verify "refs/tags/$tag" >/dev/null && { echo "$tag already exists." >&2; exit 1; }

previous=$(git describe --tags --abbrev=0 2>/dev/null || true)

# The version, everywhere it's read from.
sed -i '' "s/^version = \".*\"/version = \"$version\"/" Cargo.toml
cargo update -q --workspace
git commit -q -am "chore: release $version"
git tag "$tag"

pnpm dist:dmg
sparkle=$(sh scripts/sparkle.sh)

# What changed, from the commit subjects since the last release: features and fixes.
range=${previous:+$previous..}HEAD
notes=$(git log --format='%s' "$range" | grep -E '^(feat|fix)' | sed -E 's/^[a-z]+(\([^)]*\))?!?: //' || true)
[ -n "$notes" ] || notes="Small improvements."
html=$(printf '%s\n' "$notes" | sed 's/&/\&amp;/g; s/</\&lt;/g; s/^/<li>/; s/$/<\/li>/' | tr -d '\n')
markdown=$(printf '%s\n' "$notes" | sed 's/^/- /')

date=$(LC_ALL=C date -u '+%a, %d %b %Y %H:%M:%S +0000')
assets=""
for kind in apple-silicon intel; do
  dmg="target/Akyba-$version-$kind.dmg"
  # Sparkle's signature over the disk image, from the key in the keychain.
  signature=$("$sparkle/bin/sign_update" "$dmg")
  url="https://github.com/itsjonasarden/akyba/releases/download/$tag/Akyba-$version-$kind.dmg"
  cat > "target/appcast-$kind.xml" <<XML
<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
  <channel>
    <title>Akyba</title>
    <item>
      <title>Akyba $version</title>
      <pubDate>$date</pubDate>
      <sparkle:version>$version</sparkle:version>
      <sparkle:shortVersionString>$version</sparkle:shortVersionString>
      <sparkle:minimumSystemVersion>13.0</sparkle:minimumSystemVersion>
      <description><![CDATA[<ul>$html</ul>]]></description>
      <enclosure url="$url" type="application/octet-stream" $signature />
    </item>
  </channel>
</rss>
XML
  # The same disk image under a name without the version, so the README's download links
  # (releases/latest/download/Akyba-<kind>.dmg) always get the newest.
  cp "$dmg" "target/Akyba-$kind.dmg"
  assets="$assets $dmg target/Akyba-$kind.dmg target/appcast-$kind.xml"
done

git push -q origin main "$tag"
# Word-split on purpose: the paths have no spaces.
# shellcheck disable=SC2086
gh release create "$tag" $assets --title "Akyba $version" --notes "$markdown"
echo "Released Akyba $version: every installed Akyba offers it within a day."
