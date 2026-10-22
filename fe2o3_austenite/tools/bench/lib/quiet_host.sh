# Quiet-host helper for a timed run. Sourced, not executed.
#
# Austenite's lane builds through a slot helper that holds one of two flock files,
# `evslot1.lock` and `evslot5.lock`, for the life of each build. A timed run takes both,
# so no Austenite build starts beside it, and gives them up when it returns, so the
# lane's builds run in the gaps between timed runs. The lock files are only ever opened
# for append here: never deleted or replaced, which would orphan a held lock.
QUIET_LOCK_DIR="${QUIET_LOCK_DIR:-$HOME/.cache/cargo-targets/claude-rc-3}"

# Runs "$@" in a subshell that holds both slot locks. Blocks until it has both.
with_quiet_host() {
	(
		exec 7>>"$QUIET_LOCK_DIR/evslot1.lock"
		exec 8>>"$QUIET_LOCK_DIR/evslot5.lock"
		flock 7
		flock 8
		# The command itself does not inherit the descriptors; the subshell keeps the locks.
		"$@" 7>&- 8>&-
	)
}
