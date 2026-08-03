#!/usr/bin/env bash
#
# Regenerate every packaged icon and every piece of installer art from the SVG
# masters in `branding/`.
#
#   ./scripts/generate-icons.sh
#
# `branding/wobu-icon.svg` and `branding/wobu-mark.svg` are the only artwork
# under version control that is edited by hand; everything in `src-tauri/icons/`
# and `branding/installer/` is output.
#
# Tauri's own generator produces the rasters rather than ImageMagick because it
# is what writes a real multi-resolution `.ico` and `.icns` — a renamed PNG
# passes `file` and then fails on Windows and macOS. Wobu is desktop-only, so the
# Android and iOS asset catalogues Tauri also emits are deleted rather than
# committed.
#
# The installer art is composed from those rasters with ImageMagick. That step is
# skipped when ImageMagick is missing, because the results are committed and only
# need regenerating when the mark itself changes.
#
# Re-running this always shows `icon.icns` as changed even when the artwork has
# not: Tauri encodes the icns members in parallel and writes them in whatever
# order they finish, so the bytes shuffle while the contents stay identical.
# Everything else here is reproducible.

set -Eeuo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "$script_dir/.." && pwd)"

icon_master="$repo_root/branding/wobu-icon.svg"
mark_master="$repo_root/branding/wobu-mark.svg"
icons_out="$repo_root/src-tauri/icons"
installer_out="$repo_root/branding/installer"
tauri_bin="$repo_root/node_modules/.bin/tauri"

for master in "$icon_master" "$mark_master"; do
  if [[ ! -f "$master" ]]; then
    echo "Missing icon master at $master." >&2
    exit 1
  fi
done

if [[ ! -x "$tauri_bin" ]]; then
  echo "The Tauri CLI is not installed. Run 'npm ci' from the repository root first." >&2
  exit 1
fi

cd "$repo_root"

# ── app icons ────────────────────────────────────────────────────────────────
"$tauri_bin" icon "$icon_master" --output "$icons_out"
rm -rf "$icons_out/android" "$icons_out/ios"
echo
echo "src-tauri/icons: $(find "$icons_out" -type f | wc -l) files from branding/wobu-icon.svg"

# ── installer art ────────────────────────────────────────────────────────────
if command -v magick >/dev/null 2>&1; then
  im=(magick)
elif command -v convert >/dev/null 2>&1; then
  im=(convert)
else
  echo "ImageMagick not found — leaving branding/installer as it is." >&2
  exit 0
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# Transparent renders of the mark, sized for each surface.
"$tauri_bin" icon "$mark_master" --output "$work" -p 40 -p 96 -p 128 >/dev/null

mkdir -p "$installer_out"

# The tile gradient from the icon master, so every surface a user meets during
# an install is the same dark-to-warm wash.
tile="gradient:#232833-#0d0e12"

# NSIS header: 150x57, shown on every page but the first and last.
"${im[@]}" -size 150x57 "$tile" \
  "$work/40x40.png" -geometry +14+9 -composite \
  -type TrueColor BMP3:"$installer_out/nsis-header.bmp"

# NSIS sidebar: 164x314, the welcome and finish pages.
"${im[@]}" -size 164x314 "$tile" \
  "$work/96x96.png" -geometry +34+96 -composite \
  -type TrueColor BMP3:"$installer_out/nsis-sidebar.bmp"

# WiX banner: 493x58, the top strip of every MSI page after the first.
"${im[@]}" -size 493x58 "$tile" \
  "$work/40x40.png" -geometry +16+9 -composite \
  -type TrueColor BMP3:"$installer_out/wix-banner.bmp"

# WiX dialog: 493x312, the MSI welcome and exit pages. The right half is covered
# by text, so the mark stays on the left.
"${im[@]}" -size 493x312 "$tile" \
  "$work/96x96.png" -geometry +38+58 -composite \
  -type TrueColor BMP3:"$installer_out/wix-dialog.bmp"

# DMG background: 660x400, matching the bundler's default window. The mark sits
# above the drag target rather than behind it, so the app and Applications icons
# stay legible.
"${im[@]}" -size 660x400 "$tile" \
  "$work/128x128.png" -geometry +266+34 -composite \
  -depth 8 -strip "$installer_out/dmg-background.png"

echo "branding/installer: $(find "$installer_out" -type f | wc -l) files from branding/wobu-mark.svg"
