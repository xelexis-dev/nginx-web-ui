#!/bin/sh
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$here/guard.sh"
main=${1:?DEBIAN_SNAPSHOT required}
security=${2:?DEBIAN_SECURITY_SNAPSHOT required}
root=${3:-/}
suite=${4:-bookworm}
case "$suite" in bookworm|trixie) ;; *) pin_failure 'apt/snapshot/suite' ;; esac
for timestamp in "$main" "$security"; do
  printf '%s\n' "$timestamp" | grep -Eq '^[0-9]{8}T[0-9]{6}Z$' || pin_failure 'apt/snapshot/timestamp'
done
mkdir -p "$root/etc/apt/sources.list.d" "$root/etc/apt/apt.conf.d"
# Only replace inherited apt source entries, as specified for this build stage.
rm -f "$root/etc/apt/sources.list" "$root/etc/apt/sources.list.d/"*.list "$root/etc/apt/sources.list.d/"*.sources
cat > "$root/etc/apt/sources.list.d/nwui-snapshot.sources" <<SOURCES
Types: deb
URIs: http://snapshot.debian.org/archive/debian/$main
Suites: $suite $suite-updates
Components: main
Signed-By: /usr/share/keyrings/debian-archive-keyring.gpg
Check-Valid-Until: no

Types: deb
URIs: http://snapshot.debian.org/archive/debian-security/$security
Suites: $suite-security
Components: main
Signed-By: /usr/share/keyrings/debian-archive-keyring.gpg
Check-Valid-Until: no
SOURCES
cat > "$root/etc/apt/apt.conf.d/80nwui-snapshot" <<'APT'
Acquire::Retries "5";
Acquire::Queue-Mode "host";
Acquire::http::Pipeline-Depth "2";
APT
if grep -ER 'deb\.debian\.org|security\.debian\.org|trusted[=: ]+yes|AllowInsecure|AllowDowngrade' "$root/etc/apt/sources.list.d" "$root/etc/apt/apt.conf.d/80nwui-snapshot"; then
  pin_failure 'apt/snapshot/live-or-insecure-source'
fi
