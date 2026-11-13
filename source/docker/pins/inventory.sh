#!/bin/sh
set -eu
export LC_ALL=C
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$here/guard.sh"
stage=${1:?stage required}; output=${2:?output required}; baseline=${3:?base inventory required}; shift 3
arch=$(dpkg --print-architecture)
tmp=$(mktemp -d); trap 'rm -rf -- "$tmp"' EXIT HUP INT TERM
"$here/base-inventory.sh" > "$tmp/installed"
# apt's print-uris output is: 'URI' local-filename size checksum. No shell eval.
awk '$1 ~ /^\047https?:\/\// {uri=$1;gsub(/^\047|\047$/, "",uri);print $2 "\t" uri}' "$@" > "$tmp/uris"
printf 'binary\tarch\tversion\tsource\tsourceVersion\torigin\n' > "$output"
while IFS="$(printf '\t')" read -r package pkgarch version source source_version; do
  row=$(printf '%s\t%s\t%s\t%s\t%s' "$package" "$pkgarch" "$version" "$source" "$source_version")
  if grep -Fxq "$row" "$baseline"; then
    origin=base
  else
    origin=$(awk -F '\t' -v p="$package" -v a="$pkgarch" -v v="$version" '
      BEGIN {sub(/^[0-9]+:/,"",v)}
      {n=split($1,f,"_"); if(n!=3||f[1]!=p||f[3]!=a ".deb")next;
       ver=f[2];gsub(/%3[aA]/,":",ver);gsub(/%2[bB]/,"+",ver);sub(/^[0-9]+:/,"",ver);if(ver!=v)next;
       if($2~/^https:\/\/nginx.org\/packages\/debian\//) origins["nginx.org"]=1;
       else if($2~/^http:\/\/snapshot.debian.org\/archive\/debian-security\//)origins["debian-security"]=1;
       else if($2~/^http:\/\/snapshot.debian.org\/archive\/debian\//)origins["debian"]=1;
       else if((p=="postgresql-15"||p=="postgresql-client-15")&&v=="15.19-1.pgdg13+2"&&$2=="https://apt.postgresql.org/pub/repos/apt/pool/main/p/postgresql-15/" p "_" v "_" a ".deb")origins["pgdg"]=1;
       else origins["invalid"]=1; }
      END {for(o in origins){count++;value=o};if(count==1&&value!="invalid")print value;else exit 1}' "$tmp/uris") || pin_failure "$stage/$arch/$package-origin"
    [ -n "$origin" ] || pin_failure "$stage/$arch/$package-origin"
    # Keep the signed-index policy proof alongside transaction-derived origin.
    apt-cache policy "$package" >> "$tmp/policy"
  fi
  printf '%s\t%s\n' "$row" "$origin" >> "$output"
done < "$tmp/installed"
