#!/bin/bash
# Render each `.grad` fixture to the `.png` beside it, using a browser's SVG implementation.
#
# The PNGs are the oracle for `tests/gradient_oracle.rs` and are committed alongside the data.
# Nothing here runs during a test: regenerate only when a fixture changes, and look at the result
# before keeping it.
#
# A `.grad` file is a handful of lines both sides read, so that no part of the description is
# written twice. Blank lines and lines beginning with `#` are comments.
#
#   linear x0 y0 x1 y1      the gradient's axis, in the 256 by 256 plane
#   radial cx cy r          the gradient's centre and radius, in the same plane
#   stop t rrggbbaa         a stop at position t, colour as eight hexadecimal digits
#   rect x y w h            the rectangle to fill
#
# The page is drawn on a transparent background, so the alpha channel of the PNG is the fixture's
# own alpha and not a composite against anything.
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SIZE=256

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

for g in "$DIR"/*.grad; do
	name="$(basename "$g" .grad)"
	kind=""
	attrs=""
	stops=""
	rect=""
	while read -r verb rest; do
		case "$verb" in
			\#*|"")	continue ;;
			linear)
				set -- $rest
				kind="linearGradient"
				attrs="x1=\"$1\" y1=\"$2\" x2=\"$3\" y2=\"$4\""
				;;
			radial)
				set -- $rest
				kind="radialGradient"
				attrs="cx=\"$1\" cy=\"$2\" r=\"$3\" fx=\"$1\" fy=\"$2\""
				;;
			stop)
				set -- $rest
				# Eight hexadecimal digits: six of colour and two of alpha, which SVG carries
				# as a separate attribute rather than in the colour.
				rgb="${2:0:6}"
				a8="${2:6:2}"
				alpha="$(printf '%d' "0x$a8")"
				alpha="$(awk -v a="$alpha" 'BEGIN { printf "%.6f", a / 255 }')"
				stops="$stops<stop offset=\"$1\" stop-color=\"#$rgb\" stop-opacity=\"$alpha\"/>"
				;;
			rect)
				set -- $rest
				rect="<rect x=\"$1\" y=\"$2\" width=\"$3\" height=\"$4\" fill=\"url(#g)\"/>"
				;;
		esac
	done < "$g"

	cat > "$TMP/$name.svg" <<EOF
<svg xmlns="http://www.w3.org/2000/svg" width="$SIZE" height="$SIZE" viewBox="0 0 $SIZE $SIZE">
<defs><$kind id="g" gradientUnits="userSpaceOnUse" $attrs>$stops</$kind></defs>
$rect
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
