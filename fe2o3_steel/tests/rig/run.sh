#!/usr/bin/env bash
#
# Stands a real Steel up in a temporary directory, drives it over HTTPS with a
# real dashboard session, and tears it down. See README.md.
#
#   fe2o3_steel/tests/rig/run.sh          # run it
#   RIG_PORT=9444 fe2o3_steel/tests/rig/run.sh
#   RIG_KEEP=1 fe2o3_steel/tests/rig/run.sh   # leave the directory behind
#
# Exits non-zero if any check fails.

set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
PORT="${RIG_PORT:-9443}"
PASS='rig-test-passphrase-not-a-secret'
B="https://localhost:$PORT"

RIG_DIR="$(mktemp -d -t steel-rig-XXXXXX)"
export RIG_DIR
J="$RIG_DIR/jar"

cleanup() {
    # `exec` below makes STEEL_PID the server itself rather than the subshell
    # that launched it, so this reaches the thing that holds the port. Killing a
    # subshell leaves its child serving, which is how a test harness ends up
    # squatting on 9443 long after it claimed to have finished.
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
has()   { if echo "$2" | grep -q "$3"; then ok "$1"; else no "$1" "did not contain '$3'"; fi; }
hasnt() { if echo "$2" | grep -q "$3"; then no "$1" "contained '$3'"; else ok "$1"; fi; }

echo "== building =="
cargo build --release -p oxedyne_fe2o3_steel --bin steel --manifest-path "$ROOT/Cargo.toml" \
    2>&1 | grep -E "^error|Finished" | tail -1
[ -x "$ROOT/target/release/steel" ] || { echo "no binary"; exit 1; }

echo "== laying out $RIG_DIR =="
mkdir -p "$RIG_DIR/www/public/content/posts" "$RIG_DIR/www/src/styles"
cp "$ROOT/target/release/steel" "$RIG_DIR/steel"
printf '# The first post\n\nWritten in a directory, imported into a store.\n' \
    > "$RIG_DIR/www/public/content/posts/2026-07-01-from-the-dir.md"
sed -e "s|@PORT@|$PORT|g" "$HERE/config.jdat.in" > "$RIG_DIR/config.jdat"

# Wallet creation wants a terminal, not a pipe. See README.
echo "== wallet =="
python3 "$HERE/make_wallet.py" > "$RIG_DIR/wallet.out" 2>&1
[ -f "$RIG_DIR/wallet.jdat" ] || { echo "no wallet; see $RIG_DIR/wallet.out"; RIG_KEEP=1; exit 1; }
echo "  made"

# `-d` or the first run refuses production mode and exits 0 without saying why.
# The fifo keeps stdin open: the server keeps a shell beside the listener, and an
# EOF there ends the process. Dev mode also generates the self-signed cert.
echo "== starting =="
mkfifo "$RIG_DIR/ctl"
sleep 600 > "$RIG_DIR/ctl" &
HOLD_PID=$!
( cd "$RIG_DIR" && STEEL_ADMIN_PASS="$PASS" exec ./steel server -d < ctl > server.log 2>&1 ) &
STEEL_PID=$!

for _ in $(seq 1 30); do
    sleep 1
    curl -sk -o /dev/null --max-time 2 "$B/admin/login" && break
done
if ! curl -sk -o /dev/null --max-time 2 "$B/admin/login"; then
    echo "server did not come up; see $RIG_DIR/server.log"
    RIG_KEEP=1
    exit 1
fi
echo "  up on $PORT"

echo
echo "== the gate, before signing in =="
check "an anonymous GET is not served the composer" \
    "$(curl -sk -o /dev/null -w '%{http_code}' "$B/admin/publish")" "303"
check "an anonymous POST is refused (404, not 401)" \
    "$(curl -sk -o /dev/null -w '%{http_code}' -X POST -d 'slug=sneaky&source=x' "$B/admin/publish/save")" "404"

echo
echo "== the cookie the dashboard issues =="
# The composer lives under /admin because of what this cookie says, and for no
# other reason. If this ever stops saying Path=/admin, the design moved.
hdrs=$(curl -sk -D - -o /dev/null -X POST -d "passphrase=$PASS" "$B/admin/login")
has "signing in sets a session cookie" "$hdrs" "[Ss]et-[Cc]ookie"
has "the cookie is Path=/admin -- the whole reason the composer lives there" "$hdrs" "Path=/admin"
has "the cookie is SameSite=Strict" "$hdrs" "SameSite=Strict"
has "the cookie is HttpOnly" "$hdrs" "HttpOnly"

curl -sk -c $J -o /dev/null -X POST -d "passphrase=$PASS" "$B/admin/login"

echo
echo "== signed in =="
body=$(curl -sk -b $J "$B/admin/publish")
has "the composer serves its page" "$body" "Posts"
has "the nav carries a Publish entry" "$body" "/admin/publish"
has "the empty store says so" "$body" "Nothing written yet"

echo
echo "== import the directory =="
check "the import redirects back to the list" \
    "$(curl -sk -b $J -o /dev/null -w '%{http_code}' -X POST "$B/admin/publish/import")" "303"
body=$(curl -sk -b $J "$B/admin/publish")
has "the imported post is listed by its own heading" "$body" "The first post"
has "the imported post is live" "$body" "live"
has "the imported post kept its date" "$body" "2026-07-01"

echo
echo "== write a post =="
check "saving redirects back to the list" "$(curl -sk -b $J -o /dev/null -w '%{http_code}' -X POST \
    --data-urlencode 'slug=written-here' \
    --data-urlencode 'date=2026-07-17' \
    --data-urlencode 'kind=essay' \
    --data-urlencode 'state=draft' \
    --data-urlencode 'source=# Written in the composer

A paragraph, and a [link](https://example.com).' \
    "$B/admin/publish/save")" "303"
body=$(curl -sk -b $J "$B/admin/publish")
has "the new post is listed by its heading" "$body" "Written in the composer"
has "it is a draft" "$body" "draft"
has "it is an essay" "$body" "essay"

echo
echo "== a draft is served to nobody =="
check "the draft 404s to a reader" \
    "$(curl -sk -o /dev/null -w '%{http_code}' "$B/posts/written-here")" "404"
body=$(curl -sk "$B/posts")
hasnt "the draft is not on the public index" "$body" "Written in the composer"
has "the live post is" "$body" "The first post"

echo
echo "== but its author can preview it =="
body=$(curl -sk -b $J "$B/admin/publish/preview?slug=written-here")
has "the preview renders the prose" "$body" "Written in the composer"
has "the preview renders the Markdown link as a link" "$body" 'href="https://example.com"'
has "the preview says it is a draft" "$body" "served to nobody"

echo
echo "== publish it =="
curl -sk -b $J -o /dev/null -X POST \
    --data-urlencode 'slug=written-here' --data-urlencode 'was=written-here' \
    --data-urlencode 'date=2026-07-17' --data-urlencode 'kind=essay' \
    --data-urlencode 'state=live' \
    --data-urlencode 'source=# Written in the composer

A paragraph, and a [link](https://example.com).' \
    "$B/admin/publish/save"
check "the published post is served to a reader" \
    "$(curl -sk -o /dev/null -w '%{http_code}' "$B/posts/written-here")" "200"
body=$(curl -sk "$B/posts/written-here")
has "the reader gets the prose in the first response" "$body" "A paragraph"
has "the page carries its Open Graph card" "$body" "og:title"

echo
echo "== a form does not get to invent a key =="
# A slug reaches a database key and a URL. This is the check that matters most.
check "a slug with a path in it is refused" "$(curl -sk -b $J -o /dev/null -w '%{http_code}' -X POST \
    --data-urlencode 'slug=../../publish/index' --data-urlencode 'source=x' \
    "$B/admin/publish/save")" "303"
body=$(curl -sk -b $J "$B/admin/publish")
hasnt "and wrote nothing" "$body" "publish/index"
has "the index still serves the posts it had" "$(curl -sk "$B/posts")" "The first post"

echo
echo "== rename takes the post with it =="
curl -sk -b $J -o /dev/null -X POST \
    --data-urlencode 'slug=renamed' --data-urlencode 'was=written-here' \
    --data-urlencode 'date=2026-07-17' --data-urlencode 'kind=essay' \
    --data-urlencode 'state=live' --data-urlencode 'source=# Renamed

Words.' \
    "$B/admin/publish/save"
check "the post is at its new name" \
    "$(curl -sk -o /dev/null -w '%{http_code}' "$B/posts/renamed")" "200"
check "and no longer at the old one" \
    "$(curl -sk -o /dev/null -w '%{http_code}' "$B/posts/written-here")" "404"

echo
echo "== delete =="
curl -sk -b $J -o /dev/null -X POST --data-urlencode 'slug=renamed' "$B/admin/publish/delete"
check "a deleted post is gone" \
    "$(curl -sk -o /dev/null -w '%{http_code}' "$B/posts/renamed")" "404"

echo
echo "== a deleted post stays deleted =="
# Database::delete marks a key, and a marked key still comes back from a scan. A
# repair that trusted the scan would resurrect every post ever deleted.
curl -sk -b $J -o /dev/null -X POST "$B/admin/publish/import"
body=$(curl -sk -b $J "$B/admin/publish")
hasnt "an import does not resurrect it" "$body" "Renamed"
has "and the directory's post is still there" "$body" "The first post"

echo
echo "== a post says which kind it is, or a page cannot tell =="
# Without this the stream shows a passing thought and an essay identically, which
# is the furniture a note is defined by not wearing.
json=$(curl -sk "$B/posts/index.json")
has "the JSON carries the kind" "$json" '"kind"'
has "the imported post is a note" "$json" '"kind": "note"'

echo
echo "== two posts in one day have an order =="
# A day is not an order: same-day posts used to fall back to sorting by slug,
# which is alphabetical and has nothing to do with which was written first.
for t in "09:00|zulu-early" "14:30|alpha-late"; do
    curl -sk -b $J -o /dev/null -X POST \
        --data-urlencode "slug=${t##*|}" \
        --data-urlencode "date=2026-07-20 ${t%%|*}" \
        --data-urlencode 'kind=note' --data-urlencode 'state=live' \
        --data-urlencode "source=# ${t##*|}

Words." "$B/admin/publish/save"
done
# alpha-late is later in the day and alphabetically first. Order by time, and it
# leads; order by slug, and it leads for the wrong reason -- so check the one
# whose slug sorts *last* comes second.
order=$(curl -sk "$B/posts/index.json" | grep -o '"slug": "[a-z-]*"' | head -2 | tr '\n' ' ')
check "the later post of the day comes first" "$order" '"slug": "alpha-late" "slug": "zulu-early" '
has "a timed date survives the round trip" "$(curl -sk "$B/posts/index.json")" '"date": "2026-07-20T14:30"'
has "and a reader is given it without the T" "$(curl -sk "$B/posts/index.json")" '"date_text": "2026-07-20 14:30"'
has "the feed dates it to the minute, not to midnight" "$(curl -sk "$B/posts/feed.xml")" '2026-07-20T14:30:00Z'
has "the page shows a reader the readable form" "$(curl -sk "$B/posts/alpha-late")" '2026-07-20 14:30</time>'
has "and the machine-readable form in the attribute" "$(curl -sk "$B/posts/alpha-late")" 'datetime="2026-07-20T14:30"'

echo
echo "== a date that is not one is refused =="
check "a date with no shape is refused" "$(curl -sk -b $J -o /dev/null -w '%{http_code}' -X POST \
    --data-urlencode 'slug=badly-dated' --data-urlencode 'date=yesterday' \
    --data-urlencode 'source=x' "$B/admin/publish/save")" "303"
hasnt "and wrote nothing" "$(curl -sk -b $J "$B/admin/publish")" "badly-dated"

echo
echo "== the dashboard reads a query =="
# The path and the query are parsed apart. Reading the query out of the path
# finds nothing, silently, and the database page did exactly that for months.
body=$(curl -sk -b $J "$B/admin/database?prefix=publish/index&limit=2")
has "the prefix box echoes what was asked" "$body" 'id="prefix" name="prefix" value="publish/index"'
has "the limit box echoes what was asked" "$body" 'value="2"'
hasnt "and the scan actually filtered" "$body" "publish/post/from-the-dir"

echo
echo "$pass passed, $fail failed"
[ $fail -eq 0 ]
