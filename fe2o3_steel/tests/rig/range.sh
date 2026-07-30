#!/usr/bin/env bash
#
# Stands a real Steel up over HTTPS, serves a ten megabyte file of known bytes,
# and asks curl for windows of it. Every body is compared against the same window
# cut out of the file with dd, byte for byte, so a window taken from the wrong
# offset fails rather than merely looking plausible.
#
#   fe2o3_steel/tests/rig/range.sh              # run it
#   RIG_PORT=9445 fe2o3_steel/tests/rig/range.sh
#   RIG_KEEP=1 fe2o3_steel/tests/rig/range.sh   # leave the directory behind
#
# Exits non-zero if any check fails. See README.md for what the rig knows about
# starting a Steel that a first reading of the code does not tell you.

set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
PORT="${RIG_PORT:-9445}"
PASS='rig-test-passphrase-not-a-secret'
B="https://localhost:$PORT"
TARGET="${CARGO_TARGET_DIR:-$ROOT/target}"

# Ten megabytes, and a byte pattern with a period that shares no factor with any
# power of two -- so a window read from the wrong offset holds different bytes
# rather than the same repeating ones.
SIZE=10485760
PERIOD=251

RIG_DIR="$(mktemp -d -t steel-range-XXXXXX)"
export RIG_DIR

cleanup() {
    [ -n "${STEEL_PID:-}" ] && kill "$STEEL_PID" 2>/dev/null
    [ -n "${HOLD_PID:-}" ] && kill "$HOLD_PID" 2>/dev/null
    [ -n "${STEEL_PID:-}" ] && wait "$STEEL_PID" 2>/dev/null
    if [ "${RIG_KEEP:-0}" = "1" ]; then
        echo "rig left at $RIG_DIR"
    else
        rm -rf "$RIG_DIR"
    fi
}
trap cleanup EXIT INT TERM

pass=0; fail=0
ok()    { pass=$((pass+1)); echo "  PASS  $1"; }
no()    { fail=$((fail+1)); echo "  FAIL  $1${2:+ -- $2}"; }
check() { if [ "$2" = "$3" ]; then ok "$1"; else no "$1" "expected '$3', got '$2'"; fi; }
has()   { if echo "$2" | grep -qi -- "$3"; then ok "$1"; else no "$1" "did not contain '$3'"; fi; }
hasnt() { if echo "$2" | grep -qi -- "$3"; then no "$1" "contained '$3'"; else ok "$1"; fi; }

MEDIA="$RIG_DIR/www/public/clip.mp4"

# One header field off a response, lower-cased, carriage return stripped.
field() { grep -i "^$2:" "$1" | tr -d '\r' | sed "s/^[^:]*: *//" | tail -1; }

# A window of the served file, cut with dd -- the oracle every body is compared
# against. `skip_bytes` and `count_bytes` make the offsets exact rather than
# block-aligned.
slice() { # slice <start> <len> <out>
    dd if="$MEDIA" of="$3" bs=65536 skip="$1" count="$2" \
        iflag=skip_bytes,count_bytes status=none
}

# GET with an optional Range, keeping the headers and the body apart, then check
# the status, the two length fields and the bytes themselves.
#
#   ranged <name> <curl range args...> -- <status> <content-range> <start> <len>
ranged() {
    local name="$1"; shift
    local args=()
    while [ "$1" != "--" ]; do args+=("$1"); shift; done
    shift
    local want_status="$1" want_cr="$2" start="$3" len="$4"

    local h="$RIG_DIR/h.$$" b="$RIG_DIR/b.$$" want="$RIG_DIR/w.$$"
    local code
    code=$(curl -sk $HTTPARG "${args[@]}" -D "$h" -o "$b" -w '%{http_code}' "$B/clip.mp4")
    check "$name: status" "$code" "$want_status"

    local cr; cr=$(field "$h" content-range)
    check "$name: content-range" "$cr" "$want_cr"

    local cl; cl=$(field "$h" content-length)
    check "$name: content-length" "$cl" "$len"

    has "$name: advertises byte ranges" "$(field "$h" accept-ranges)" "bytes"

    local got; got=$(stat -c %s "$b")
    check "$name: body length" "$got" "$len"

    if [ "$len" -gt 0 ]; then
        slice "$start" "$len" "$want"
        if cmp -s "$want" "$b"; then ok "$name: body matches the file byte for byte"
        else no "$name: body differs from the file"; fi
        rm -f "$want"
    fi
    rm -f "$h" "$b"
}

echo "== building =="
cargo build --release -p oxedyne_fe2o3_steel --bin steel --manifest-path "$ROOT/Cargo.toml" \
    2>&1 | grep -E "^error|Finished" | tail -1
[ -x "$TARGET/release/steel" ] || { echo "no binary at $TARGET/release/steel"; exit 1; }

echo "== laying out $RIG_DIR =="
mkdir -p "$RIG_DIR/www/public" "$RIG_DIR/www/src/styles"
cp "$TARGET/release/steel" "$RIG_DIR/steel"
sed -e "s|@PORT@|$PORT|g" "$HERE/range_config.jdat.in" > "$RIG_DIR/config.jdat"

# The file under test: a counting pattern, so every window is distinguishable
# from every other one. Named `.mp4` because the content type a recording is
# served under is part of what makes a browser offer to seek in it.
python3 - "$MEDIA" "$SIZE" "$PERIOD" <<'PY'
import sys
path, size, period = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
pattern = bytes(range(period))
whole = (pattern * (size // period + 1))[:size]
with open(path, "wb") as f:
    f.write(whole)
PY
: > "$RIG_DIR/www/public/empty.mp4"
printf 'x' > "$RIG_DIR/www/public/one.mp4"
# A page worth encoding, for the coding checks: markup is highly redundant, so
# the encoded form is a fraction of the weight and the two lengths cannot be
# confused for each other.
MARKUP="$RIG_DIR/www/public/page.html"
python3 - "$MARKUP" <<'PY'
import sys
line = "<p>a paragraph of markup that repeats and so encodes well</p>\n"
with open(sys.argv[1], "w") as f:
    f.write("<!DOCTYPE html>\n<html><body>\n")
    f.write(line * 4000)
    f.write("</body></html>\n")
PY
PAGE=$(stat -c %s "$MARKUP")
echo "  $(stat -c %s "$MEDIA") bytes at clip.mp4, $PAGE bytes at page.html"

echo "== wallet =="
python3 "$HERE/make_wallet.py" > "$RIG_DIR/wallet.out" 2>&1
[ -f "$RIG_DIR/wallet.jdat" ] || { echo "no wallet; see $RIG_DIR/wallet.out"; RIG_KEEP=1; exit 1; }
echo "  made"

echo "== starting =="
mkfifo "$RIG_DIR/ctl"
sleep 900 > "$RIG_DIR/ctl" &
HOLD_PID=$!
( cd "$RIG_DIR" && STEEL_ADMIN_PASS="$PASS" exec ./steel server -d < ctl > server.log 2>&1 ) &
STEEL_PID=$!

for _ in $(seq 1 40); do
    sleep 1
    curl -sk -o /dev/null --max-time 2 "$B/clip.mp4" -r 0-0 && break
done
if ! curl -sk -o /dev/null --max-time 5 -r 0-0 "$B/clip.mp4"; then
    echo "server did not come up; see $RIG_DIR/server.log"
    RIG_KEEP=1
    exit 1
fi
echo "  up on $PORT"

echo
echo "== what the connection actually negotiated =="
# Steel offers `http/1.1` over ALPN and nothing else, so `--http2` negotiates
# 1.1 and falls back. The sweep is repeated under it regardless: the file layer
# must behave identically whichever way the client asked.
for arg in "" "--http2"; do
    v=$(curl -sk $arg -o /dev/null -w '%{http_version}' "$B/clip.mp4" -r 0-0)
    echo "  curl ${arg:-(default)} -> HTTP/$v"
done

for HTTPARG in "" "--http2"; do
    label="${HTTPARG:-http1.1}"
    echo
    echo "== the sweep, ${label} =="

    ranged "whole file"       -- 200 "" 0 $SIZE
    ranged "first hundred"    -r 0-99      -- 206 "bytes 0-99/$SIZE" 0 100
    ranged "from a hundred"   -r 100-      -- 206 "bytes 100-$((SIZE-1))/$SIZE" 100 $((SIZE-100))
    ranged "last fifty"       -r -50       -- 206 "bytes $((SIZE-50))-$((SIZE-1))/$SIZE" $((SIZE-50)) 50
    ranged "one byte"         -r 0-0       -- 206 "bytes 0-0/$SIZE" 0 1
    ranged "the last byte"    -r $((SIZE-1))- \
        -- 206 "bytes $((SIZE-1))-$((SIZE-1))/$SIZE" $((SIZE-1)) 1
    # The seek a player actually makes: a window from the middle, which is the
    # case an off-by-one in the offset makes plausible rubbish of.
    ranged "a window in the middle" -r 5242880-5243391 \
        -- 206 "bytes 5242880-5243391/$SIZE" 5242880 512
    # More than there is, from a start that exists: clamped, not refused.
    ranged "an end past the end" -r $((SIZE-10))-99999999 \
        -- 206 "bytes $((SIZE-10))-$((SIZE-1))/$SIZE" $((SIZE-10)) 10
    # A suffix longer than the file is the whole file.
    ranged "a suffix longer than the file" -r -99999999 \
        -- 206 "bytes 0-$((SIZE-1))/$SIZE" 0 $SIZE

    echo
    echo "  -- the refusals, ${label} --"
    h="$RIG_DIR/h.$$"

    code=$(curl -sk $HTTPARG -r 999999999- -D "$h" -o /dev/null -w '%{http_code}' "$B/clip.mp4")
    check "a start past the end: status" "$code" "416"
    check "a start past the end: content-range" "$(field "$h" content-range)" "bytes */$SIZE"

    code=$(curl -sk $HTTPARG -H "Range: bytes=-0" -D "$h" -o /dev/null -w '%{http_code}' \
        "$B/clip.mp4")
    check "the last nothing: status" "$code" "416"
    check "the last nothing: content-range" "$(field "$h" content-range)" "bytes */$SIZE"

    code=$(curl -sk $HTTPARG -r 0-99 -D "$h" -o /dev/null -w '%{http_code}' "$B/empty.mp4")
    check "an empty file satisfies nothing: status" "$code" "416"
    check "an empty file satisfies nothing: content-range" \
        "$(field "$h" content-range)" "bytes */0"

    echo
    echo "  -- what is ignored rather than refused, ${label} --"
    # RFC 9110 §14.2: a unit we do not implement, and a field that does not
    # parse, are ignored -- the client gets the file, not a rejection.
    for bad in "items=0-9" "bytes=99-10" "bytes=abc-def" "bytes=" "chunks=0-"; do
        code=$(curl -sk $HTTPARG -H "Range: $bad" -D "$h" -o /dev/null -w '%{http_code}' \
            "$B/clip.mp4")
        check "'$bad' is ignored, not refused" "$code" "200"
        check "'$bad' gets the whole file" "$(field "$h" content-length)" "$SIZE"
    done
    # Several ranges at once: recognised, and answered with the whole file
    # rather than a multipart body.
    code=$(curl -sk $HTTPARG -H "Range: bytes=0-49,100-149" -D "$h" -o /dev/null \
        -w '%{http_code}' "$B/clip.mp4")
    check "several ranges are answered whole" "$code" "200"
    check "and carry the whole length" "$(field "$h" content-length)" "$SIZE"
    hasnt "and no multipart body" "$(field "$h" content-type)" "multipart"

    echo
    echo "  -- HEAD, ${label} --"
    hd=$(curl -sk $HTTPARG -I "$B/clip.mp4")
    has "HEAD advertises byte ranges" "$hd" "accept-ranges: bytes"
    has "HEAD states the full length" "$hd" "content-length: $SIZE"
    has "HEAD names the recording's type" "$hd" "content-type: video/mp4"
    body=$(curl -sk $HTTPARG -I -o "$RIG_DIR/hb.$$" -w '%{size_download}' "$B/clip.mp4")
    check "HEAD sends no body" "$body" "0"
    # RFC 9110 §14.2 defines range handling for GET alone and requires a server
    # to ignore the field on any other method. So a HEAD carrying a Range is
    # answered about the whole file: a 206 there would name a window nobody can
    # read and understate the size of the thing being asked about.
    hd=$(curl -sk $HTTPARG -I -H "Range: bytes=0-99" -D - -o /dev/null "$B/clip.mp4")
    has "a ranged HEAD ignores the range" "$hd" "200 OK"
    hasnt "and names no window" "$hd" "content-range"
    has "and states the full length" "$hd" "content-length: $SIZE"

    echo
    echo "  -- a HEAD of something worth encoding, ${label} --"
    # The GET is encoded, and says so.
    hd=$(curl -sk $HTTPARG -H "Accept-Encoding: gzip" -D - -o /dev/null "$B/page.html")
    has "a GET that accepts gzip is encoded" "$hd" "content-encoding: gzip"
    has "and says it varies by coding" "$hd" "vary: accept-encoding"
    # The HEAD is not: encoding it would mean reading and compressing the whole
    # page to throw the result away. So it reports the identity length, which is
    # what a GET accepting no coding would be told, and names no coding it has
    # not applied.
    hd=$(curl -sk $HTTPARG -I -H "Accept-Encoding: gzip" -D - -o /dev/null "$B/page.html")
    has "a HEAD that accepts gzip is not encoded" "$hd" "200 OK"
    hasnt "and names no coding" "$hd" "content-encoding"
    has "and states the identity length" "$hd" "content-length: $PAGE"
    has "and still says it varies by coding" "$hd" "vary: accept-encoding"
    # The property a live monitor rests on: a plain HEAD is the GET's byte count.
    hd=$(curl -sk $HTTPARG -I -D - -o /dev/null "$B/page.html")
    has "a plain HEAD states the GET's byte count" "$hd" "content-length: $PAGE"

    echo
    echo "  -- a one byte file, ${label} --"
    code=$(curl -sk $HTTPARG -r 0- -D "$h" -o /dev/null -w '%{http_code}' "$B/one.mp4")
    check "the only byte: status" "$code" "206"
    check "the only byte: content-range" "$(field "$h" content-range)" "bytes 0-0/1"
    code=$(curl -sk $HTTPARG -r 1- -D "$h" -o /dev/null -w '%{http_code}' "$B/one.mp4")
    check "one past it: status" "$code" "416"

    rm -f "$h" "$RIG_DIR/hb.$$"
done

echo
echo "== the connection survives a window =="
# A body shorter or longer than the Content-Length promised desynchronises every
# message after it, which only shows up when two requests share a connection.
# `--next` starts a second request on the same connection rather than a second
# connection, which is the only way the desynchronisation shows.
two=$(curl -sk -r 0-99 -o "$RIG_DIR/a1" -w '%{http_code} ' "$B/clip.mp4" \
      --next -sk -r 200-299 -o "$RIG_DIR/a2" -w '%{http_code}' "$B/clip.mp4")
check "two windows down one connection" "$two" "206 206"
slice 0 100 "$RIG_DIR/e1"; slice 200 100 "$RIG_DIR/e2"
if cmp -s "$RIG_DIR/e1" "$RIG_DIR/a1" && cmp -s "$RIG_DIR/e2" "$RIG_DIR/a2"
then ok "and both windows are the right bytes"
else no "a window on a kept-alive connection came out wrong"; fi

if [ "${RIG_TRANSCRIPT:-0}" = "1" ]; then
    echo
    echo "== transcripts =="
    # The exchanges themselves, for a report or a bug: request line, request
    # fields, status line, response fields. Bodies are dropped.
    for spec in "an un-ranged GET::" \
                "the first hundred bytes:-r 0-99:" \
                "from a hundred to the end:-r 100-:" \
                "the last fifty bytes:-r -50:" \
                "a start past the end:-r 999999999-:" \
                "several ranges at once::-H Range: bytes=0-49,100-149" \
                "a HEAD:-I:"
    do
        name="${spec%%:*}"; rest="${spec#*:}"
        args="${rest%%:*}"; hdr="${rest#*:}"
        echo
        echo "--- $name ---"
        if [ -n "$hdr" ]; then
            curl -sk -o /dev/null -v $args -H "${hdr#-H }" "$B/clip.mp4" 2>&1 \
                | grep -E "^[<>] " | grep -v "^> $" | grep -v "^< $"
        else
            curl -sk -o /dev/null -v $args "$B/clip.mp4" 2>&1 \
                | grep -E "^[<>] " | grep -v "^> $" | grep -v "^< $"
        fi
    done
fi

echo
echo "== $pass passed, $fail failed =="
[ "$fail" = "0" ] || RIG_KEEP="${RIG_KEEP:-0}"
exit $([ "$fail" = "0" ] && echo 0 || echo 1)
