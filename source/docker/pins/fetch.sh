#!/bin/sh
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$here/guard.sh"
stage=${1:?stage required}; url=${2:?URL required}; output=${3:?output required}; hash=${4:?SHA256 required}
curl -fsSL --retry 5 --connect-timeout 15 --max-time 300 "$url" -o "$output" || pin_failure "$stage/$(dpkg --print-architecture)/$(basename "$output")-download"
"$here/verify-file.sh" "$stage" "$output" "$hash"
