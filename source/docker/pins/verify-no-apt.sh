#!/bin/sh
set -eu
export LC_ALL=C
fail() { echo "Runtime apt removal check failed: $*" >&2; exit 1; }
rows=$(mktemp)
trap 'rm -f -- "$rows"' EXIT HUP INT TERM
dpkg-query -W -f='${binary:Package}\t${db:Status-Status}\n' > "$rows" || fail 'cannot enumerate package state'
[ -s "$rows" ] || fail 'empty package state'
awk -F '\t' 'NF!=2 {exit 1} {sub(/:[^:]*$/, "", $1); if (($1=="apt" || $1~/^libapt-pkg/) && $2!="not-installed") exit 1}' "$rows" \
  || fail 'apt package state remains'
for tool in apt apt-get apt-cache apt-cdrom apt-config apt-mark; do
  if command -v "$tool" >/dev/null 2>&1; then fail "$tool is executable"; fi
  if [ -e "/usr/bin/$tool" ] || [ -L "/usr/bin/$tool" ]; then fail "/usr/bin/$tool remains"; fi
done
for path in /usr/lib/apt /usr/lib/dpkg/methods/apt /etc/cron.daily/apt /etc/cron.daily/apt-compat \
  /usr/lib/systemd/system/apt-* /lib/systemd/system/apt-* /etc/systemd/system/timers.target.wants/apt-*; do
  if [ -e "$path" ] || [ -L "$path" ]; then fail "$path remains"; fi
done
for directory in /usr/lib /lib /usr/lib64 /lib64 /usr/local/lib; do
  for path in "$directory"/libapt-pkg.so* "$directory"/libapt-private.so* \
    "$directory"/*/libapt-pkg.so* "$directory"/*/libapt-private.so*; do
    if [ -e "$path" ] || [ -L "$path" ]; then fail "$path remains"; fi
  done
done
