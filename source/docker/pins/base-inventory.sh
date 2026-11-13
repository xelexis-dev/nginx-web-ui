#!/bin/sh
set -eu
export LC_ALL=C
# source:* fields include dpkg's fallback to the binary identity/version.
dpkg-query -W -f='${db:Status-Status}\t${binary:Package}\t${Architecture}\t${Version}\t${source:Package}\t${source:Version}\n' \
  | awk -F '\t' 'BEGIN {OFS="\t"} $1=="installed" {sub(/:[^:]*$/, "", $2); print $2,$3,$4,$5,$6}' | sort
