#!/bin/sh
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$here/guard.sh"
stage=${1:?stage required}; file=${2:?file required}; hash=${3:-}
arch=$(dpkg --print-architecture)
printf '%s\n' "$hash" | grep -Eq '^[0-9a-f]{64}$' || pin_failure "$stage/$arch/$(basename "$file")-sha256"
printf '%s  %s\n' "$hash" "$file" | sha256sum -c - || pin_failure "$stage/$arch/$(basename "$file")"
