#!/bin/sh
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$here/guard.sh"
stage=${1:?stage required}; actual=${2:?actual required}; expected=${3:?expected required}
arch=$(dpkg --print-architecture)
[ -f "$expected" ] || pin_failure "$stage/$arch/inventory-missing"
if ! cmp -s "$expected" "$actual"; then
  diff -u "$expected" "$actual" >&2 || true
  pin_failure "$stage/$arch/inventory"
fi
