#!/bin/sh
set -eu
. /etc/s6-overlay/scripts/nwui-gate.sh
nwui_gate_oneshot
. /etc/s6-overlay/scripts/pg-env.sh
# The runner provisioned and verified PG17 while holding both storage locks.
[ "$(cat "$PGDATA/PG_VERSION")" = 17 ] || { echo storage_selection_invalid >&2; exit 2; }
