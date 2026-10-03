# Sourced by the gate scripts. The one way they run a typst 0.15.x: under the memory cap, and with the
# account's own fonts hidden. Typst looks for them under `$XDG_DATA_HOME/fonts`; a face installed there (a
# second Libertinus Serif of another version, say) outranks Typst's own and would move a glyph on one side
# only, so `XDG_DATA_HOME` names a directory that does not exist. This is the shell twin of
# `typst_command` in `tests/eval_oracle/oracle.rs`.

GATE_CAP="${GATE_CAP:-3G}"

# Runs a command under the cap, in the claude-rc slice.
gate_cap() {
	systemd-run --user --scope --quiet -p "MemoryMax=${GATE_CAP}" --slice=claude-rc.slice "$@"
}

# gate_typst <typst> <args...>. GATE_SCRATCH names a directory that exists; its child `no-user-fonts` must not.
gate_typst() {
	local bin="${1:?gate_typst: no typst}"
	shift
	local none="${GATE_SCRATCH:?gate_typst: no scratch}/no-user-fonts"
	if [ -e "$none" ]; then
		echo "gate_typst: the font isolation directory exists" >&2
		return 97
	fi
	XDG_DATA_HOME="$none" gate_cap "$bin" "$@"
}
