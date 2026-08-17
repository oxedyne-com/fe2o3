#!/usr/bin/env bash
# Puts a .xlsx this crate wrote to an external reader, and says what came back.
#
# A spreadsheet is the format where reading your own output proves least. The things that separate it
# from a table -- a string that lives in another part, a date that is only a date because a style says
# so, a formula whose cached value is what everyone sees -- are all CONVENTIONS rather than structure,
# so a file can satisfy this crate's own reader completely and still open in Excel as five columns of
# five-digit integers.
#
# LibreOffice is the second opinion. It is not Excel, and where the two disagree the right answer is
# unknown and the right response is to preserve rather than to choose. But it is somebody else's
# implementation, which is the only kind of evidence there is here.
#
#   fe2o3_file/dev/xlsx_oracle.sh
#
# Needs `soffice` on the path.

set -eu

work="$(mktemp -d "${XDG_CACHE_HOME:-$HOME/.cache}/xlsx-oracle.XXXXXX")"
trap 'rm -rf "$work"' EXIT

root="$(cd "$(dirname "$0")/../.." && pwd)"
target="${CARGO_TARGET_DIR:-$HOME/.cache/cargo-targets/${RC_SLOT:-solo}/fe2o3}"

CARGO_TARGET_DIR="$target" cargo run -q -p oxedyne_fe2o3_file --example make_xlsx \
	--manifest-path "$root/Cargo.toml" -- "$work/made.xlsx"

echo "-- the archive --"
unzip -t "$work/made.xlsx"

echo "-- what LibreOffice made of the cells --"
HOME="$work" soffice --headless --convert-to csv --outdir "$work" "$work/made.xlsx" >/dev/null
cat "$work/made.csv"

echo "-- and of the things a CSV cannot show --"
HOME="$work" soffice --headless --convert-to fods --outdir "$work" "$work/made.xlsx" >/dev/null
echo -n "sheets:    "; grep -o 'table:name="[^"]*"' "$work/made.fods" | tr '\n' ' '; echo
echo -n "formulas:  "; grep -o 'table:formula="[^"]*"' "$work/made.fods" | tr '\n' ' '; echo
echo -n "types:     "; grep -o 'office:value-type="[^"]*"' "$work/made.fods" \
	| sort | uniq -c | tr '\n' ' '; echo
# A date that arrives as `float` is the whole failure this file exists to catch: it means the number
# reached the reader and the style that made it a date did not.
echo -n "dates:     "; grep -o 'office:date-value="[^"]*"' "$work/made.fods" | tr '\n' ' '; echo
