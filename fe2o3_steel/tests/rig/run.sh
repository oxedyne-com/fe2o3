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
echo "== /admin and /admin/ both reach the dashboard =="
# A trailing slash is not an unknown sub-route. Both are the root, both redirect
# an unauthenticated visitor to the login form rather than 404ing one of them.
check "/admin redirects to login" \
    "$(curl -sk -o /dev/null -w '%{http_code}' "$B/admin")" "303"
loc=$(curl -sk -D - -o /dev/null "$B/admin" | grep -io "location: [^[:space:]]*" | tr -d '\r')
has "and the login is where it points" "$loc" "/admin/login"
check "/admin/ with a slash is the same, not a 404" \
    "$(curl -sk -o /dev/null -w '%{http_code}' "$B/admin/")" "303"
has "the login form is served" "$(curl -sk "$B/admin/login")" "passphrase"

echo
echo "== the operator dashboard still works, and is a separate tier =="
# The operator login is not the site console's login. It stays Path=/admin and
# SameSite=Strict; the console is reached with a member's Path=/ cookie instead.
hdrs=$(curl -sk -D - -o /dev/null -X POST -d "passphrase=$PASS" "$B/admin/login")
has "the operator login sets a session cookie" "$hdrs" "[Ss]et-[Cc]ookie"
has "scoped to /admin, not the whole site" "$hdrs" "Path=/admin"
has "and SameSite=Strict" "$hdrs" "SameSite=Strict"
curl -sk -c $J -o /dev/null -X POST -d "passphrase=$PASS" "$B/admin/login"
has "the operator reaches the dashboard" "$(curl -sk -b $J "$B/admin")" "Overview"
hasnt "and content authoring has left the dashboard" "$(curl -sk -b $J "$B/admin")" "/admin/publish"

echo
echo "== the site console, driven as a member admin over its own login =="
# The whole point: a member on the site's list manages the site from within it,
# with a member session, never touching the operator dashboard or the wallet.
# WebSocket login plus HTTP console -- so a small node driver, not curl.
RIG_PORT="$PORT" RIG_PASS="rig member admin passphrase not a secret" \
    node --experimental-websocket "$HERE/console_rig.mjs" 2>&1 | grep -v "ExperimentalWarning\|--trace-warnings"
console_status=${PIPESTATUS[0]}
if [ "$console_status" = "0" ]; then ok "the console rig passed"; else no "the console rig failed"; fi

echo
echo "== the dashboard reads a query =="
# The path and the query are parsed apart. Reading the query out of the path
# finds nothing, silently, and the database page did exactly that for months.
body=$(curl -sk -b $J "$B/admin/database?prefix=publish/index&limit=2")
has "the prefix box echoes what was asked" "$body" 'id="prefix" name="prefix" value="publish/index"'
has "the limit box echoes what was asked" "$body" 'value="2"'
hasnt "and the scan actually filtered" "$body" "publish/post/from-the-dir"

echo
echo "== the reports page =="
# Reports read the subscriber store and the send history and aggregate them. A
# site that has sent nothing must say so rather than draw an empty table, and the
# page is a read behind the same gate as the rest of the console.
MJ="$RIG_DIR/mjar"
curl -sk -c $MJ -o /dev/null -X POST -d "passphrase=$PASS" "$B/manage/login"
anon=$(curl -sk "$B/manage/reports")
has "anonymous gets the login, not the numbers" "$anon" 'name="passphrase"'
# The class names all appear in the stylesheet every console page inlines, so a
# leak shows as a class being *used*, not merely defined.
hasnt "and no subscriber figures leak to it" "$anon" 'class="mc-stat-n"'
body=$(curl -sk -b $MJ "$B/manage/reports")
has "an admin gets the reports page" "$body" "<h1>Reports</h1>"
has "it reports on the list" "$body" "The list"
has "and on the sends" "$body" "Newsletter sends"
has "an empty list says so" "$body" "Nobody has subscribed yet"
has "and an unsent newsletter says so" "$body" "No post has been mailed"
hasnt "rather than drawing an empty table" "$body" 'class="mc-bar-fill"'
has "the console links to it" "$(curl -sk -b $MJ "$B/manage")" "/manage/reports"

echo
echo "== read counts =="
# A read is counted when a post is served to somebody who is neither the author
# nor an obvious machine. All three of those exclusions can only be proved from
# outside, by actually fetching the post as each of them in turn.
BROWSER='Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36'
body=$(curl -sk -b $MJ "$B/manage/reports")
has "an unread site says so" "$body" "Nothing has been read yet"

# curl announces itself, and is therefore not a reader. Ten fetches, no count.
for _ in $(seq 1 10); do curl -sk -o /dev/null "$B/posts/from-the-dir"; done
body=$(curl -sk -b $MJ "$B/manage/reports")
has "ten fetches by a machine count for nothing" "$body" "Nothing has been read yet"

# The author, carrying a management session, is not a reader of their own post.
curl -sk -b $MJ -o /dev/null -A "$BROWSER" "$B/posts/from-the-dir"
body=$(curl -sk -b $MJ "$B/manage/reports")
has "nor does the author reading their own post" "$body" "Nothing has been read yet"

# A browser with no management session is a reader, and is counted.
curl -sk -o /dev/null -A "$BROWSER" "$B/posts/from-the-dir"
body=$(curl -sk -b $MJ "$B/manage/reports")
hasnt "a reader is counted" "$body" "Nothing has been read yet"
has "and the post is named with its tally" "$body" "from-the-dir"

# The index is not a post, so browsing it does not count as reading everything on it.
before=$(curl -sk -b $MJ "$B/manage/reports")
curl -sk -o /dev/null -A "$BROWSER" "$B/posts"
curl -sk -o /dev/null -A "$BROWSER" "$B/posts/feed.xml"
after=$(curl -sk -b $MJ "$B/manage/reports")
if [ "$before" = "$after" ]; then ok "the index and the feed are not reads"
else no "the index or the feed counted as a read"; fi

# The page states what it cannot know, rather than leaving the absence to be read
# as an oversight.
has "the page says a read is not a reader" "$after" "a reading, not a reader"

echo
echo "== no management surface invites a crawler =="
# A login page indexes nothing worth having and advertises where the dashboard
# is. The console had this and the dashboard did not, which is how oxedyne.com's
# /admin/login came to be indexable. Both are pages a crawler can reach without
# a session, so both must say so themselves.
has "the dashboard login says noindex" "$(curl -sk "$B/admin/login")" 'name="robots" content="noindex"'
has "and the console login too" "$(curl -sk "$B/manage")" 'name="robots" content="noindex"'
has "as does the console itself" "$(curl -sk -b $MJ "$B/manage")" 'name="robots" content="noindex"'
has "and the dashboard itself" "$(curl -sk -b $J "$B/admin")" 'name="robots" content="noindex"'
# The prose is the opposite case: it exists to be found, and must not be told
# otherwise by a stray blanket rule.
hasnt "the posts stay findable" "$(curl -sk "$B/posts")" 'name="robots" content="noindex"'

echo
echo "== the JSON an app draws its own console from =="
# The app's Manage tab draws the subscribers and the reports itself rather than
# opening a page of the server's, so both must be available as data and both must
# be behind the same gate as the pages.
anon=$(curl -sk "$B/manage/subscribers.json")
hasnt "anonymous gets no subscriber data" "$anon" '"subscribers"'
anon=$(curl -sk "$B/manage/reports.json")
hasnt "anonymous gets no report data" "$anon" '"reads"'
body=$(curl -sk -b $MJ "$B/manage/subscribers.json")
has "an admin gets the subscriber list as JSON" "$body" '"subscribers"'
has "with the counts beside it" "$body" '"confirmed"'
body=$(curl -sk -b $MJ "$B/manage/reports.json")
has "an admin gets the reports as JSON" "$body" '"list"'
has "including the sends" "$body" '"sends"'
has "and the reads" "$body" '"reads"'
has "the reads name each post" "$body" '"from-the-dir"'

echo
echo "== the destinations page =="
# The server-rendered twin of the app's Destinations panel: the only one a site
# without the app has. Every secret is write-only, so a stored secret must never
# come back down the wire -- which is the whole point of the page and the one
# thing a check from outside can prove.
anon=$(curl -sk "$B/manage/destinations")
has "anonymous gets the login, not the settings" "$anon" 'name="passphrase"'
hasnt "and no destination form leaks to it" "$anon" 'name="dest" value="mastodon"'
body=$(curl -sk -b $MJ "$B/manage/destinations")
has "an admin gets the destinations page" "$body" "<h1>Destinations</h1>"
has "it offers Mastodon" "$body" 'name="dest" value="mastodon"'
has "and Bluesky" "$body" 'name="dest" value="bluesky"'
has "each form carries the write token" "$body" 'name="csrf"'
has "a secret field is masked" "$body" 'type="password"'
has "and never autofilled from the browser" "$body" 'autocomplete="new-password"'
has "an unset remote says so" "$body" "Not set."
hasnt "and offers nothing to clear" "$body" 'name="clear" value="1"'
has "the console links to it" "$(curl -sk -b $MJ "$B/manage")" "/manage/destinations"

echo
echo "== the editor is an editor, not a form with three verbs =="
# The editor's only verb is Save: leaving is a close, and deleting belongs beside the post in
# the list. It carries a live preview pane, so the separate preview page is not the only way
# to see the prose rendered.
ed=$(curl -sk -b $MJ "$B/manage/edit?slug=from-the-dir")
has "the editor has a live preview pane" "$ed" 'id="mc-preview"'
has "and posts its source to the renderer" "$ed" "/manage/render"
has "leaving is a close in the corner" "$ed" 'class="mc-close"'
hasnt "not a Cancel button" "$ed" ">Cancel<"
hasnt "and there is no Delete in the editor" "$ed" 'class="mc-btn mc-btn-danger"'
has "Save is the one verb" "$ed" ">Save</button>"

echo
echo "== the list copes with more than fits on a screen =="
# A filter and a pager, and a delete beside each post rather than buried in the editor.
ls=$(curl -sk -b $MJ "$B/manage")
has "the list can be searched" "$ls" 'name="q"'
has "and filtered by state" "$ls" 'name="state"'
has "each row can be deleted, with a confirm" "$ls" "There is no undo"
has "and deleting is an icon, not a word" "$ls" "mc-ico-danger"
has "the reader's view is an icon too" "$ls" 'class="mc-ico"'
one=$(curl -sk -b $MJ "$B/manage?q=zzzznothingmatchesthis")
has "a search that matches nothing says so" "$one" "No post matches that"

echo
echo "$pass passed, $fail failed"

# A check reads the markup; only a browser renders it, and a defect class that
# hides behind correct markup -- a modifier that never applies, a control row of
# stepped heights -- is visible nowhere else. `RIG_HOLD=1` keeps the server up so
# a browser can be pointed at it, rather than tearing down the one thing worth
# looking at. Ctrl-C ends it, and cleanup still runs.
if [ "${RIG_HOLD:-0}" = "1" ]; then
    echo
    echo "holding at $B (passphrase: $PASS)"
    echo "the manage session cookie jar is at $MJ"
    echo "Ctrl-C to stop"
    wait "$STEEL_PID" 2>/dev/null || true
fi

[ $fail -eq 0 ]
