#!/usr/bin/env bash
# Minimal callsite proof only: recorder refusal never owns command execution/status.
recorder=$1
shift
handle=$("$recorder" record-start tsc-request verify) || handle=
"$@"
status=$?
boundary=request
if [ "$status" -eq 0 ]; then boundary=none; fi
if [ -n "$handle" ]; then
    "$recorder" record-finish "$handle" "$status" "$boundary" || :
fi
exit "$status"
