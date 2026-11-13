#!/bin/sh
set -eu
. /etc/s6-overlay/scripts/nwui-gate.sh
nwui_gate_oneshot
# The runner owns paired secrets and key publication; never re-lock or regenerate.
