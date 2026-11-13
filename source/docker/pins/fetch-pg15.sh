#!/bin/sh
# Only the two reviewed PGDG artifacts; all dependencies resolve from Debian snapshots.
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$here/guard.sh"
arch=${1:?}; version=${2:?}; server=${3:?}; client=${4:?}; out=${5:?}
case "$arch:$version" in amd64:15.19-1.pgdg13+2|arm64:15.19-1.pgdg13+2) ;; *) pin_failure 'pg15/unsupported-artifact';; esac
mkdir -p "$out"
: > "$out/transaction.txt"
for package in postgresql-15 postgresql-client-15; do
  case "$package" in postgresql-15) hash=$server;; *) hash=$client;; esac
  file="${package}_${version}_${arch}.deb"
  url="https://apt.postgresql.org/pub/repos/apt/pool/main/p/postgresql-15/$file"
  "$here/fetch.sh" pg15 "$url" "$out/$file" "$hash"
  printf "'%s' %s %s SHA256:%s\n" "$url" "$file" "$(wc -c < "$out/$file" | tr -d ' ')" "$hash" >> "$out/transaction.txt"
done
