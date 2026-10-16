#!/usr/bin/env bash
# tick.sh <wall-file> <command...>: runs the command and writes its wall time in seconds to <wall-file>, read
# from bash's microsecond clock around it, so neither systemd-run nor this shell's own start is counted.
w="$1"
shift
s=$EPOCHREALTIME
"$@"
rc=$?
e=$EPOCHREALTIME
awk -v s="$s" -v e="$e" 'BEGIN { printf "%.4f\n", e - s }' > "$w"
exit $rc
