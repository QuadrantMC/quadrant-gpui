#!/usr/bin/env bash
# Runs a command until it succeeds, up to 5 tries with a growing pause.
# Release steps download tools from GitHub releases, which answer 504 now and
# then; a retry gets past it.
set -u
for try in 1 2 3 4 5; do
  "$@" && exit 0
  status=$?
  if [ "$try" -lt 5 ]; then
    echo "::warning::try $try of '$*' failed ($status), retrying in $((try * 20))s"
    sleep $((try * 20))
  fi
done
exit "$status"
