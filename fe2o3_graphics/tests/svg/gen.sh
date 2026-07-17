#!/bin/bash
# Render each `.path` fixture to the `.png` beside it, using a browser's SVG implementation.
#
# The PNGs are the oracle for `tests/svg_oracle.rs` and are committed alongside the data. Nothing
# here runs during a test: regenerate only when a fixture changes, and look at the result before
# keeping it.
#
# Each `.path` file is a viewBox on its first line, as `# minx miny width height`, and the path data
# on the rest. That split is what lets the fixture be shared: the browser reads it as an SVG
# attribute and the test reads the identical bytes into its own parser. No XML is involved on either
# side.
#
# The page is drawn on a transparent background with a black fill, so the alpha channel of the PNG
# *is* the coverage the renderer computed, with no luminance conversion to argue about.
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SIZE=256

# A browser that is not confined by a snap sandbox is preferred, since a snap cannot write outside
# $HOME and this directory may sit anywhere.
CHROME=""
for c in \
	"$HOME/.cache/ms-playwright/chromium-1229/chrome-linux64/chrome" \
	"$HOME/.cache/ms-playwright/chromium-1228/chrome-linux64/chrome" \
	"$(command -v google-chrome || true)" \
	"$(command -v chromium || true)"
do
	if [ -n "$c" ] && [ -x "$c" ]; then
		CHROME="$c"
		break
	fi
done
if [ -z "$CHROME" ]; then
	echo "No Chrome or Chromium found to render with." >&2
	exit 1
fi
echo "Rendering with: $CHROME"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

for p in "$DIR"/*.path; do
	name="$(basename "$p" .path)"
	view="$(head -1 "$p" | sed 's/^#[[:space:]]*//')"
	data="$(tail -n +2 "$p")"
	cat > "$TMP/$name.svg" <<EOF
<svg xmlns="http://www.w3.org/2000/svg" width="$SIZE" height="$SIZE" viewBox="$view">
<path d="$data" fill="#000"/>
</svg>
EOF
	"$CHROME" --headless=new --no-sandbox --disable-gpu --hide-scrollbars \
		--force-device-scale-factor=1 --default-background-color=00000000 \
		--window-size="$SIZE,$SIZE" --virtual-time-budget=3000 \
		--screenshot="$TMP/$name.png" "file://$TMP/$name.svg" >/dev/null 2>&1 || true
	if [ ! -f "$TMP/$name.png" ]; then
		echo "  FAILED to render $name" >&2
		exit 1
	fi
	mv "$TMP/$name.png" "$DIR/$name.png"
	echo "  $name.png"
done
echo "Done. Look at the PNGs before keeping them."
