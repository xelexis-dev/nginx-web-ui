#!/bin/sh
# Only the runner-selected PG17 layout serves traffic; PG15 stays private to upgrades.
if [ "${PGROOT:-/var/lib/postgresql/data}" != /var/lib/postgresql/data ] ||
   [ "${PGDATA:-/var/lib/postgresql/data/17/data}" != /var/lib/postgresql/data/17/data ]; then
  echo 'storage_override: 기존 PGDATA 설정이 새 저장 경로와 다릅니다. 데이터를 옮기지 말고 2.0 업그레이드 안내를 확인하세요.' >&2
  exit 2
fi
PGROOT=/var/lib/postgresql/data
PGDATA=$PGROOT/17/data
PGBIN=/usr/lib/postgresql/17/bin
export PGROOT PGDATA PGBIN
