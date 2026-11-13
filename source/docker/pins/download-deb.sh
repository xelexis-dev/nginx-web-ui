#!/bin/sh
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$here/guard.sh"
stage=${1:?stage required}; spec=${2:?package=version required}
apt-get download "$spec" || pin_failure "$stage/$(dpkg --print-architecture)/$spec"
