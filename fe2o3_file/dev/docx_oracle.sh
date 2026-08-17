#!/usr/bin/env bash
# Puts a .docx this crate wrote to an external reader, and says what came back.
#
# A test that reads back what this crate wrote proves that this crate agrees with itself, which is
# worth very little for a format whose whole purpose is to be opened by somebody else's program.
# LibreOffice is that somebody else. It is not Microsoft Word -- where the two disagree, the right
# answer is unknown and the right response is to preserve rather than to choose -- but it is a second
# implementation's opinion, which is the only kind of evidence there is here.
#
#   fe2o3_file/dev/docx_oracle.sh <markdown file>
#
# Needs `soffice` on the path.

set -eu

md="${1:?usage: docx_oracle.sh <markdown file>}"
# Not /tmp. LibreOffice writes a whole user profile into HOME, and on a machine where /tmp is a tmpfs
# that profile is held in RAM and charged to whoever ran this.
work="$(mktemp -d "${XDG_CACHE_HOME:-$HOME/.cache}/docx-oracle.XXXXXX")"
trap 'rm -rf "$work"' EXIT

root="$(cd "$(dirname "$0")/../.." && pwd)"
target="${CARGO_TARGET_DIR:-$HOME/.cache/cargo-targets/${RC_SLOT:-solo}/fe2o3}"

CARGO_TARGET_DIR="$target" cargo run -q -p oxedyne_fe2o3_file --example make_docx \
	--manifest-path "$root/Cargo.toml" -- "$md" "$work/made.docx"

echo "-- the archive --"
unzip -t "$work/made.docx"

echo "-- what LibreOffice read back --"
HOME="$work" soffice --headless --convert-to txt:Text --outdir "$work" "$work/made.docx" >/dev/null
cat "$work/made.txt"

echo "-- the structure it recovered --"
HOME="$work" soffice --headless --convert-to fodt --outdir "$work" "$work/made.docx" >/dev/null
echo -n "heading levels: "; grep -o 'text:outline-level="[0-9]"' "$work/made.fodt" | sort -u | tr '\n' ' '; echo
echo -n "links: ";          grep -o 'xlink:href="[^"]*"' "$work/made.fodt" | tr '\n' ' '; echo
echo -n "alignments: ";     grep -o 'fo:text-align="[^"]*"' "$work/made.fodt" | sort | uniq -c | tr '\n' ' '; echo
