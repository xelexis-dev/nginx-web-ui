#!/bin/sh
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$here/guard.sh"
record=${1:?transaction record required}; shift
[ "$#" -gt 0 ] || pin_failure 'apt/install/empty-request'
# Resolve the exact signed-index source before installation; this records the
# transaction URI even if two snapshots advertise the same binary version.
apt-get --print-uris -y --no-install-recommends install "$@" > "$record" || pin_failure "apt/install/$*"
apt-get install -y --no-install-recommends "$@" || pin_failure "apt/install/$*"
