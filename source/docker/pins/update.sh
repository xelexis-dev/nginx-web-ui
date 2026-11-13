#!/bin/sh
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$here/guard.sh"
apt-get update || pin_failure "${1:?stage required}/$(dpkg --print-architecture)/signed-index"
