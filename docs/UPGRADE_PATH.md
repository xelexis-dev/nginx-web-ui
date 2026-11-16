# 업그레이드 경로

대상 버전: **2.0.3**

2.0.3은 관리 화면의 배치와 안내를 정리한 버전입니다. 다시 발행한 2.0.0과 2.0.2와 DB 구조가 같으며, 이들 버전에서 올릴 때 데이터 이전은 없습니다. Debian 13·PostgreSQL 17(PG17)을 사용합니다. **1.19.x는 기존 DB·앱 볼륨과 설정을 유지하고 이미지 태그를 바꿔 시작하면 PostgreSQL 15(PG15) 이전을 자동으로 진행합니다. 1.19.0–1.19.11에는 필수 중간 버전이나 별도의 수동 이전이 없습니다.** 이전 중에는 관리 포트에 상태 페이지만 표시되고 프록시 사이트는 중지되므로 점검 시간을 확보하세요.

2.0.0은 다시 발행한 이미지입니다. 처음 배포한 2.0.0과 그 뒤의 2.0.1의 변경을 모두 담고 패스키 로그인과 관리 화면 프록시 수정을 더했으며, 2.0.1은 이 2.0.0에 합쳐졌습니다. 이전 2.0.0이나 2.0.1을 설치했다면 아래 「재발행 전 2.0.0·2.0.1에서」를 확인하세요.

변경 내용은 [업데이트 내역](https://github.com/xelexis-dev/nginx-web-ui/blob/main/docs/UPDATE.md), 설치 설정 전문은 [설치 안내](https://github.com/xelexis-dev/nginx-web-ui/blob/main/README.md)를 확인하세요.

**1.19.11이 1.x의 마지막 버전입니다.** 이후 1.x 릴리스(보안 수정 포함)는 없으며 새 수정은 2.0 계열에만 나옵니다. `latest` 태그는 1.19.11에 머물고 메이저 버전을 넘어 이동하지 않으므로, 2.0.3은 버전 태그로 지정해 교체하세요.

정상 경로 요약:

1. 커널 5.11 이상(Linux)과 여유 공간을 확인하고, 상태 점검 자동 재시작 도구를 끕니다.
2. 컨테이너를 정상 종료하고 DB·앱 데이터를 같은 시점의 쌍으로 백업합니다.
3. 같은 볼륨·아키텍처를 유지한 채 이미지 주소만 2.0.3으로 바꿔 시작합니다.
4. 관리 포트의 진행 상태가 끝날 때까지 기다립니다. 이 동안 프록시 사이트는 중지됩니다.
5. 버전 2.0.3, 기존 계정 로그인, 실제 프록시 응답을 확인합니다.
6. 문제가 있으면 아래 「3. 실패·보류와 재시도」와 「4. 1.19.11로 되돌리기」를 따릅니다. 성공 후 1.19.11로 돌아가면 그 사이의 모든 쓰기를 잃습니다.


## 지원 경로

| 현재 설치 | 2.0.3으로 가는 방법 | 필수 중간 버전 |
| --- | --- | --- |
| Linux 커널 5.11 미만 또는 필요한 시스템 호출이 차단된 환경 | 2.0.3 시작 불가 · 커널·Docker 엔진을 갱신할 때까지 1.19.11 유지. 1.19.11 이후 1.x 수정은 없음 | 지원 환경 필요 |
| 1.19.x · PG15 | DB·앱 데이터를 함께 백업하고 같은 볼륨으로 이미지 교체. 처음 시작할 때 자동 이전. 재발행 전 2.0.0에서 「저장소 확인」에 멈춘 설치도 같음 | 없음 |
| 다시 발행한 2.0.0과 2.0.2 · PG17 | 같은 볼륨·아키텍처를 유지하고 태그 `2.0.3`으로 교체. DB 구조 변경과 데이터 이전 없음 | 없음 |
| 재발행 전 2.0.0·2.0.1 · PG17 | 같은 볼륨·아키텍처로 태그 `2.0.3` 이미지를 다시 받아 교체. 처음 시작할 때 패스키용 DB 구조를 추가하며 그 뒤에는 이전 이미지로 시작할 수 없음(`database_newer`) | 없음 |
| 1.18.x · PG15 | 같은 데이터 검사를 거쳐 직접 이전할 수 있지만 직접 이전 검증 범위 밖이므로 1.19.11 경유 권장 | 없음 · **1.19.11 권장** |
| 1.18.0 미만 | 먼저 1.19.11로 일반 업데이트한 뒤 2.0.3으로 자동 이전 | **1.19.11 필수** |
| 사용자가 바꾼 DB 구조 등 지원하지 않는 원본 | 거부 이유를 확인. 데이터를 초기화하거나 상태 기록을 수정하지 않음 | 지원 조건 확인 |

1.18.x의 1.19.11 경유는 **권장이며 필수가 아닙니다.** 직접 이전은 1.19.x와 같은 데이터 검사를 거치지만, 확인을 마친 직접 이전 범위는 1.19.0–1.19.11입니다.

**1.18.0 미만은 먼저 1.19.11로 업데이트한 뒤 2.0.3으로 이전해야 합니다.** 그 단계의 이미지·로그 볼륨 변경 사항은 1.19.11 당시 안내를 참고하세요. 1.19.x 설치를 이 문서 때문에 먼저 1.19.11로 올릴 필요는 없습니다. PG16 경유도 필요하지 않습니다.

이전 원본이 너무 오래돼 이전 시작을 거부하면 「먼저 1.19.11로 올린 뒤 이 이미지로 시작하세요.」라는 안내를 따르세요.

### 재발행 전 2.0.0·2.0.1에서

2.0.0을 다시 발행하기 전의 2.0.0이나 2.0.1을 설치했다면 같은 볼륨·아키텍처를 유지하고 이미지 태그를 `2.0.3`(AMD64) 또는 `2.0.3-arm64`(ARM64)로 지정합니다. 2.0.1을 쓰던 설치도 `2.0.3`으로 바꿉니다. `docker compose pull`로 새 이미지를 다시 받아야 하며, digest(`@sha256:…`)로 고정했다면 새 이미지의 digest로 바꿉니다.

시작 전에 「1. 중지·백업·공간 준비」대로 DB·앱 데이터를 같은 시점의 쌍으로 백업하세요. PG17 이전을 이미 마친 설치는 다시 이전하지 않습니다. 처음 시작할 때 패스키를 저장할 DB 구조를 추가하며, 그 뒤에는 이전 2.0.0·2.0.1 이미지가 이 DB로 시작하지 않고 `database_newer`를 표시합니다. 이전 이미지로 돌아가려면 업데이트 전에 만든 백업 쌍을 복원해야 합니다. 업데이트 후 계정 화면의 「패스키」 영역과 계정 메뉴의 버전 2.0.3을 확인하세요. 이전 2.0.0에서 「저장소 확인」에 멈춘 설치는 아래 「재발행 전 2.0.0에서 저장소 확인에 멈춘 경우」를 확인하세요.

## 1. 중지·백업·공간 준비

- **Linux 호스트는 커널 5.11 이상과 최신 Docker 엔진이 필요합니다.** Docker 서버에서 `uname -r`로 확인하세요. 낮은 커널이나 오래된 Docker의 시스템 호출 필터 때문에 `isolation_unavailable`이 발생하면 신규 설치는 로그인 화면이 열리지 않고, 1.19.x에서 업그레이드하면 PG15를 복원한 뒤 대기합니다. 지원 환경을 준비할 때까지 1.19.11을 유지하세요. macOS/Windows의 Docker Desktop에는 호스트 커널 조건이 적용되지 않습니다.
- 자동 이전 중에는 autoheal 등 상태 점검 실패로 컨테이너를 재시작하는 도구를 꺼 두세요. 중지 시 최대 600초를 기다리고 강제 종료 전에 로그에서 종료 결과를 확인합니다.
- 기존 이미지·Compose·환경 설정을 보관합니다. 인증·암호화 키와 DB 접속 비밀값이 담긴 앱 데이터는 DB와 같은 시점의 쌍으로 백업하고 복원 가능 여부를 확인하세요. 자동으로 만드는 PG15 백업은 이 독립 백업을 대신하지 않습니다.
- **같은 DB·앱 볼륨을 두 컨테이너에서 동시에 사용하지 마세요.** 1.19.x와 2.0.3을 함께 실행해 비교하는 것도 해당합니다. 비교하려면 독립된 복원 사본을 사용합니다.
- 여유 공간은 대략 <b>DB 크기의 4배 + 가장 큰 인덱스의 2배 + max(1 GiB, DB 크기의 1/5)</b>에 고정 예약 공간(DB 256 MiB·앱 16 MiB)과 앱 사본·목록 공간을 더해 준비합니다. 작은 DB도 약 1.3 GB 이상이 필요하며 inode도 확보하세요. DB·앱이 같은 파일시스템이면 두 필요량을 합산합니다. 성공 후에도 PG15 백업은 운영자가 제거할 때까지 남습니다. 상태 화면·로그에 표시된 **필요 공간(`requiredBytes`)·필요 inode와 사용 가능한 수치**를 기준으로 확보하되, 서로 다른 파일시스템이면 각각 필요한 용량·inode를 충족해야 합니다. 표시된 합계에 여유가 있어도 한쪽의 부족분을 다른 쪽 공간으로 채울 수 없으므로 공간 부족 안내를 따라 해당 저장소를 확보하세요. 독립 백업·기존 보관 사본·로그 공간은 따로 고려합니다.
- 파일 크기만으로 정확한 중단 시간을 정할 수 없습니다. 데이터·인덱스 크기와 디스크 속도에 따라 달라집니다. macOS는 호스트 여유 공간과 Docker Desktop의 디스크 한도를 모두 확인하세요.
- 호스트 로그 폴더는 root 소유로 정리됩니다. nginx 로그는 root:nginx·0750, 앱 로그는 root·0755입니다. 외부 프로그램이 로그를 쓰거나 root가 아닌 계정으로 읽는 경우 권한을 확인하세요. root로 읽는 로그 감시는 유지됩니다.


아래는 Bash 등 sh 호환 셸의 예시입니다. 컨테이너 이름과 Compose 파일명이 다르면 실제 값으로 바꿉니다.

```sh
docker image inspect "$(docker inspect nginx-web-ui --format '{{.Image}}')" --format '{{.Architecture}}'
docker inspect nginx-web-ui --format '{{json .Mounts}}'
docker compose -f compose.yaml stop -t 600
docker logs --tail=100 nginx-web-ui
```

정상 종료를 확인하고 DB·앱 데이터를 함께 백업합니다. 백업에는 파일 내용과 접근 권한을 보존하고, Linux 관리 볼륨에서는 소유자·그룹도 유지하세요. 비정상 종료한 원본을 억지로 정상 완료로 표시하거나 `PG_VERSION`·복구 기록을 수정하지 마세요. 이전 과정은 비정상 종료한 DB도 검사하고 필요한 복구를 진행합니다.

### 중지 후 DB·앱을 함께 백업하고 복원하기

정상 동작을 확인한 설치를 **완전히 중지한 뒤 DB와 앱 데이터를 같은 시점의 한 쌍으로** 보관합니다. DB에는 계정·프록시 설정 등이 있고 앱 데이터에는 인증·암호화 키 등이 있습니다. 한쪽만 복원하면 로그인하거나 저장된 비밀 값을 읽지 못할 수 있습니다. 로그는 필요하면 별도로 보관합니다.

아래 명령은 Docker를 실행하는 컴퓨터의 sh 호환 셸에서 실행합니다. 예제의 컨테이너·서비스 이름과 파일명은 실제 설치에 맞춥니다. 백업에는 비밀번호와 암호화 키가 포함되므로 보관 폴더와 파일의 접근을 제한하고 다른 저장소에도 복사해 두세요. **같은 이미지·아키텍처로 먼저 복원을 확인**하며, 이미지 업데이트와 DB 이전은 아래 2절을 따릅니다.

#### 1. 이미지·실행 설정을 보관하고 정상 종료

```sh
set -eu
NWUI_CONTAINER=nginx-web-ui
NWUI_IMAGE_ID=$(docker inspect "$NWUI_CONTAINER" --format '{{.Image}}')
NWUI_BACKUP_DIR="$PWD/nwui-backup-$(date +%Y%m%d-%H%M%S)"
umask 077
mkdir -m 700 "$NWUI_BACKUP_DIR"
cp -p compose.yaml "$NWUI_BACKUP_DIR/compose.yaml"
# 사용 중인 env_file이 더 있으면 함께 복사합니다.
if [ -f admin.env ]; then cp -p admin.env "$NWUI_BACKUP_DIR/admin.env"; fi
if [ -f .env ]; then cp -p .env "$NWUI_BACKUP_DIR/.env"; fi
printf '%s\n' "$NWUI_IMAGE_ID" > "$NWUI_BACKUP_DIR/image-id.txt"
docker image inspect "$NWUI_IMAGE_ID" > "$NWUI_BACKUP_DIR/image.json"
docker image inspect "$NWUI_IMAGE_ID" --format '{{json .RepoDigests}}' > "$NWUI_BACKUP_DIR/image-digests.json"
docker inspect "$NWUI_CONTAINER" > "$NWUI_BACKUP_DIR/container.json"

docker compose -f compose.yaml stop -t 600 nginx-web-ui
test "$(docker inspect "$NWUI_CONTAINER" --format '{{.State.Running}}')" = false
test "$(docker inspect "$NWUI_CONTAINER" --format '{{.State.ExitCode}}')" = 0
docker logs --tail=100 "$NWUI_CONTAINER"
docker inspect "$NWUI_CONTAINER" --format '{{range .Mounts}}{{println .Type .Name .Source .Destination}}{{end}}'
```

정상 종료와 DB 종료 로그를 확인한 뒤 다음으로 진행합니다. 종료 코드가 0이 아니거나 강제 종료됐다면 원인을 확인하세요. 다시 발행하기 전의 2.0.0 이미지를 중지할 때는 정상 종료여도 `Error: interrupted: 시작 준비를 중단했습니다…` 한 줄이 로그에 남을 수 있습니다. 종료 코드가 0이고 DB 종료 로그(`database system is shut down`)가 있으면 정상 종료입니다. 지금의 2.0.3은 이 오류 줄 대신 `[upgrade] 중지 요청을 받아 종료합니다.` 한 줄만 남깁니다. 같은 DB·앱 저장소에 쓰는 다른 컨테이너도 모두 중지하고, 백업이 끝날 때까지 다시 시작하지 않습니다.

`DATA_KEY_FILE`을 DB·앱 밖으로 지정했다면 **그 키 파일도 같은 백업에 반드시 포함**합니다. 다른 관리 볼륨이면 아래 관리 볼륨 명령을 한 번 더, 호스트 파일이면 아래 호스트 폴더 방식으로 키가 들어 있는 폴더를 추가 보관합니다. 복원할 때 그 파일을 원래 컨테이너 내부 경로에 연결하고 `DATA_KEY_FILE` 값도 유지해야 합니다. Compose에서 별도로 연결한 인증서·설정 파일과 셸에서 넘긴 환경 값도 보관합니다. `container.json`에는 환경 값이 들어 있으므로 공개하지 않습니다.

`image-digests.json`의 `저장소@sha256:…` 주소와 이미지 아키텍처를 보관하세요. 로컬에서만 만든 이미지라 목록이 비어 있으면 원래 이미지를 별도로 보존해야 합니다. 다른 Docker 서버에서 복원할 때는 같은 아키텍처의 기록된 이미지를 먼저 준비합니다.

#### 2. Docker 관리 볼륨 또는 Linux 호스트 폴더 백업

다음 함수는 중지된 저장소를 읽기 전용으로 연결해 내용·숨김 파일·숫자 소유자/그룹·권한·ACL·확장 속성을 보관합니다. `17/data` 같은 내부 하위 폴더가 아니라 **마운트 루트 전체**를 백업합니다.

```sh
nwui_pack() {
  docker run --rm --pull=never --network none \
    --mount "type=$1,source=$2,target=/from,readonly" \
    --mount "type=bind,source=$NWUI_BACKUP_DIR,target=/backup" \
    --entrypoint /bin/sh "$NWUI_IMAGE_ID" -ec \
    'umask 077; tar --format=pax --numeric-owner --acls --xattrs --xattrs-include="*" -czpf "/backup/$1" -C /from .; tar -tzf "/backup/$1" >/dev/null' sh "$3"
}
```

**관리 볼륨:** 실제 연결된 볼륨 이름을 읽어 사용합니다. 기본 설치에서는 `nwu-pg`, `nwu-app`입니다.

```sh
NWUI_PG_VOLUME=$(docker inspect "$NWUI_CONTAINER" --format '{{range .Mounts}}{{if and (eq .Type "volume") (eq .Destination "/var/lib/postgresql/data")}}{{.Name}}{{end}}{{end}}')
NWUI_APP_VOLUME=$(docker inspect "$NWUI_CONTAINER" --format '{{range .Mounts}}{{if and (eq .Type "volume") (eq .Destination "/var/lib/nginx-web-ui")}}{{.Name}}{{end}}{{end}}')
test -n "$NWUI_PG_VOLUME" && test -n "$NWUI_APP_VOLUME"
docker volume inspect "$NWUI_PG_VOLUME" "$NWUI_APP_VOLUME" >/dev/null
nwui_pack volume "$NWUI_PG_VOLUME" pg.tar.gz
nwui_pack volume "$NWUI_APP_VOLUME" app.tar.gz
```

**Linux 호스트 폴더:** 위 조회 결과에서 `/var/lib/postgresql/data`와 `/var/lib/nginx-web-ui`에 연결한 실제 호스트 폴더를 지정합니다. 두 폴더가 모두 존재하는지 확인합니다.

```sh
NWUI_PG_DIR=/absolute/path/nwui/postgres
NWUI_APP_DIR=/absolute/path/nwui/app
test -d "$NWUI_PG_DIR" && test -d "$NWUI_APP_DIR"
nwui_pack bind "$NWUI_PG_DIR" pg.tar.gz
nwui_pack bind "$NWUI_APP_DIR" app.tar.gz
```

외부 키 폴더가 있으면 같은 함수로 `key.tar.gz`도 만듭니다. 로그도 필요하면 같은 방식으로 추가합니다. 보관을 마친 뒤 원본 서비스를 다시 시작할 수 있습니다. 업데이트 전 백업이라면 이후의 업데이트 절차를 따릅니다.

#### 3. macOS 호스트 폴더 백업

macOS 공유 폴더는 컨테이너와 호스트에서 소유자 표시가 다를 수 있으므로 **호스트에서** 보관합니다. 원래 호스트의 숫자 ID·ACL·확장 속성을 복원할 수 있는 권한으로 실행하세요. 권한 오류를 무시하거나 소유권 보존 옵션을 빼지 않습니다.

```sh
NWUI_PG_DIR=/absolute/path/nwui/postgres
NWUI_APP_DIR=/absolute/path/nwui/app
unset COPYFILE_DISABLE
test -d "$NWUI_PG_DIR" && test -d "$NWUI_APP_DIR"
/usr/bin/tar --format pax --numeric-owner --acls --xattrs --mac-metadata -czpf "$NWUI_BACKUP_DIR/pg.tar.gz" -C "$NWUI_PG_DIR" .
/usr/bin/tar --format pax --numeric-owner --acls --xattrs --mac-metadata -czpf "$NWUI_BACKUP_DIR/app.tar.gz" -C "$NWUI_APP_DIR" .
/usr/bin/tar -tzf "$NWUI_BACKUP_DIR/pg.tar.gz" >/dev/null
/usr/bin/tar -tzf "$NWUI_BACKUP_DIR/app.tar.gz" >/dev/null
```

외부 키 폴더도 같은 옵션으로 별도 보관합니다. 파일을 읽을 권한이 부족하면 관리자 권한으로 실행합니다.

#### 4. 새 저장소로 쌍을 복원

복원할 백업 폴더와 기록한 동일한 이미지를 지정합니다. 같은 Docker 서버에 원래 이미지가 남아 있으면 `image-id.txt` 값을 사용하고, 다른 서버에서는 기록된 `저장소@sha256:…` 주소로 준비한 이미지를 지정합니다. **원래 DB·앱 저장소는 남겨 두고 빈 새 저장소로 복원**합니다. 아래에서 관리 볼륨 또는 호스트 폴더 중 실제 복원 방식을 하나만 선택합니다.

```sh
NWUI_BACKUP_DIR=/absolute/path/to/nwui-backup
NWUI_RESTORE_IMAGE=$(cat "$NWUI_BACKUP_DIR/image-id.txt")
docker image inspect "$NWUI_RESTORE_IMAGE" >/dev/null
NWUI_RESTORE_ID=$(date +%Y%m%d-%H%M%S)
nwui_unpack() {
  docker run --rm --pull=never --network none \
    --mount "type=$1,source=$2,target=/to" \
    --mount "type=bind,source=$NWUI_BACKUP_DIR,target=/backup,readonly" \
    --entrypoint tar "$NWUI_RESTORE_IMAGE" \
    --numeric-owner --same-owner --same-permissions --acls --xattrs --xattrs-include='*' \
    -xzpf "/backup/$3" -C /to
}
```

**관리 볼륨:** 새 볼륨에만 복원합니다.

```sh
NWUI_RESTORE_PG="nwu-restore-pg-$NWUI_RESTORE_ID"
NWUI_RESTORE_APP="nwu-restore-app-$NWUI_RESTORE_ID"
for NWUI_VOLUME in "$NWUI_RESTORE_PG" "$NWUI_RESTORE_APP"; do
  if docker volume inspect "$NWUI_VOLUME" >/dev/null 2>&1; then exit 1; fi
  docker volume create "$NWUI_VOLUME"
done
nwui_unpack volume "$NWUI_RESTORE_PG" pg.tar.gz
nwui_unpack volume "$NWUI_RESTORE_APP" app.tar.gz
```

**호스트 폴더:** Linux와 macOS 모두 기존 데이터와 겹치지 않는 새 DB·앱 폴더를 만듭니다.

```sh
NWUI_RESTORE_PG_DIR="$PWD/nwui-restore-pg-$NWUI_RESTORE_ID"
NWUI_RESTORE_APP_DIR="$PWD/nwui-restore-app-$NWUI_RESTORE_ID"
mkdir -m 700 "$NWUI_RESTORE_PG_DIR" "$NWUI_RESTORE_APP_DIR"
```

Linux에서는 위 함수로 복원합니다.

```sh
nwui_unpack bind "$NWUI_RESTORE_PG_DIR" pg.tar.gz
nwui_unpack bind "$NWUI_RESTORE_APP_DIR" app.tar.gz
```

macOS에서는 호스트에서 복원합니다.

```sh
unset COPYFILE_DISABLE
/usr/bin/tar --numeric-owner --same-owner --acls --xattrs --mac-metadata -xzpf "$NWUI_BACKUP_DIR/pg.tar.gz" -C "$NWUI_RESTORE_PG_DIR"
/usr/bin/tar --numeric-owner --same-owner --acls --xattrs --mac-metadata -xzpf "$NWUI_BACKUP_DIR/app.tar.gz" -C "$NWUI_RESTORE_APP_DIR"
```

외부 키도 **새 별도 저장소**에 복원합니다. 보관한 Compose와 환경 파일을 복사해 복원용 설정을 만드세요. 이미지는 기록된 동일한 이미지, `container_name`은 `nginx-web-ui-restored`처럼 다른 이름으로 정하고 DB·앱 마운트를 새 저장소로 바꿉니다. 기본 Compose의 관리 볼륨 이름은 최상위 `volumes.pg-data.name`과 `volumes.app-data.name`에 있습니다. 로그는 새 별도 저장소를 쓰고, 외부 키의 마운트 원본도 복원한 파일로 바꾸되 컨테이너 내부 경로와 `DATA_KEY_FILE` 값은 유지합니다. 포트·네트워크·기존 인증 설정은 보존합니다.

```sh
docker compose -p nwu-restore -f compose.restore.yaml config --quiet
docker compose -p nwu-restore -f compose.restore.yaml up -d
docker inspect nginx-web-ui-restored --format '{{range .Mounts}}{{println .Source .Destination}}{{end}}'
docker logs --tail=100 nginx-web-ui-restored
```

원본 컨테이너는 중지한 채 보존하고, 복원본이 **새 DB·앱 쌍과 복원한 외부 키**를 사용하는지 확인합니다. 원본을 다시 실행해 둔 경우에는 복원본과 호스트 포트가 겹치지 않게 조정합니다.

#### 5. 복원 확인

1. 별도 브라우저 창에서 기존 계정으로 새로 로그인합니다. 최초 관리자 생성 화면이 보이면 계정을 만들지 말고 데이터 마운트를 확인합니다.
2. 저장된 리버스 프록시가 보이는지 확인하고 설정을 적용한 뒤 실제 도메인·포트로 대상 서비스의 응답을 확인합니다.
3. SMTP를 사용했다면 비밀번호를 다시 입력하지 않고 저장된 값을 유지한 채 테스트 발송을 합니다. 다른 암호화된 설정을 사용했다면 그 설정의 실제 동작도 확인합니다. 비밀번호가 저장됐다는 표시만으로 복호화 성공을 판단하지 않습니다.

확인을 마치기 전에는 원본 데이터와 백업을 보존합니다.

다른 호스트로 옮길 때는 같은 아키텍처·이미지와 숫자 소유자/그룹·파일시스템의 ACL/확장 속성 지원을 확인하고 별도로 복원 검증을 합니다. 저장 공간과 소요 시간은 실제 데이터·인덱스·기존 보관 사본·저장소 속도에 따라 달라집니다.

## 2. 같은 데이터로 이미지 교체

**호스트가 아닌 기존 컨테이너 이미지의 아키텍처를 유지**합니다. 기존 이미지가 `amd64`이면 `2.0.3`, `arm64`이면 `2.0.3-arm64`입니다. ARM64 호스트에서 AMD64를 에뮬레이션으로 쓰던 설치도 기존 AMD64 이미지를 유지합니다. DB 파일을 그대로 두고 다른 아키텍처로 바꾸지는 마세요.

기존 Compose에서 이미지 주소를 바꿉니다. `…:1.19.11-arm64@sha256:…`처럼 digest로 고정했다면 태그만 바꾸지 말고 `image:`의 전체 주소를 교체하세요. 이전 digest가 남으면 이전 이미지가 실행됩니다. 관리 볼륨의 실제 이름, DB·앱 마운트, 포트, 네트워크와 인증 환경 설정을 유지하세요. 새 빈 볼륨을 만들거나 DB·앱 한쪽만 교체하지 않습니다.

```yaml
services:
  nginx-web-ui:
    image: ghcr.io/xelexis-dev/nginx-web-ui:2.0.3
    security_opt: ["no-new-privileges:true"]
    stop_grace_period: 10m
```

Compose 파일에서 컨테이너를 재생성하세요. 관리 도구의 기존 컨테이너 설정 복사 방식은 이전 이미지의 `PGDATA`·`APP_GIT_SHA` 등 환경변수까지 가져와 `storage_override` 또는 빌드 정보의 `verification_failed`로 시작이 거부될 수 있습니다. 복사된 이미지 기본 환경변수를 제거하고 운영자가 지정한 설정만 유지하세요. 사용자 지정 healthcheck는 이전 중 unhealthy를 표시할 수 있으며 서비스가 아직 시작되지 않아 생기는 정상적인 상태이므로, 이를 이유로 자동 재시작하지 않도록 하세요.

위 예시는 기존 서비스에 합칠 항목입니다. 새 Compose 전문으로 덮어쓰는 예시가 아닙니다. 컨테이너는 기본 관리자 권한으로 시작하며 관리 요청 처리만 별도 일반 사용자 권한으로 동작합니다. `user:`를 추가하거나 공유 폴더 권한을 넓히지 마세요.

```sh
docker compose -f compose.yaml config --quiet
docker compose -f compose.yaml pull
docker compose -f compose.yaml up -d
docker logs --tail=100 nginx-web-ui
```

이전 중에는 **기존 호스트 관리 포트**(기본 `http://<Docker 서버 주소>:8080`)로 직접 접속해 단계·처리량·오류와 다음 조치를 확인합니다. 같은 컨테이너의 80/443을 거치는 관리 도메인과 프록시 사이트는 이때 사용할 수 없습니다. 실행 중인 컨테이너에서도 다음 읽기 전용 명령으로 상태를 볼 수 있습니다.

```sh
docker exec nginx-web-ui nwui-db-upgrade auto-status
```

완료되면 화면 오른쪽 위 계정 버튼을 눌러 열리는 메뉴의 버전이 **2.0.3**인지 확인하고 기존 계정으로 로그인한 뒤 HTTP/HTTPS·TCP/UDP 응답, 인증서, 알림·노드 연결을 확인합니다. 기존 설치에서 최초 관리자 생성 화면이 보이면 새 계정을 만들지 말고 마운트한 데이터 쌍부터 확인하세요. 다시 시작하면 저장된 설정을 검사해 적용합니다. 대상 이름을 확인하지 못하는 등 특정 항목의 문제를 확인할 수 있으면 그 항목과 의존하는 프록시만 제외하고 관리 화면과 나머지 정상 프록시를 시작합니다. 이는 DB 이전 실패가 아니며, 제외 이유는 알림과 미적용 변경에서 확인합니다. 원인을 해결한 뒤 위쪽 막대의 「적용」을 누르거나 컨테이너를 다시 시작하면 저장된 설정을 다시 검사합니다. 다시 시작할 때 제한 시간 안에 문제 항목을 가려내지 못하면 기본 설정 검사에 통과한 경우에만 저장된 프록시 항목을 모두 제외하고 관리 화면을 시작합니다. 업데이트 중에는 이 전체 제외를 하지 않으며, 기본 설정 검사도 실패하거나 전체 설정·권한·저장소 검사에 실패하면 시작을 중단합니다. 이전 도중 중단됐다면 다음 시작에서 기록을 확인해 복구하거나 이어서 준비합니다. PG17 전환 전 복원이 필요하면 원본 PG15를 검증 후 복원하고 그 시작에서는 대기하며, PG17 전환 후에는 PG17을 유지한 채 시작을 이어갑니다. 표시된 다음 조치를 확인하고 복구 기록을 지우지 마세요.

기존 관리 화면 프록시에 저장된 X-Real-IP·X-Forwarded-Proto는 실행할 때 자동 값으로 대체됩니다. 새 프록시 화면이 미리 채우는 값(`$remote_addr`, `$scheme`)은 그대로 저장할 수 있습니다. 해당 경로를 편집해 저장할 때 두 헤더에 자동 값과 다른 값이 남아 있어 저장이 거부되면 두 항목을 지우거나 자동 값으로 바꾼 뒤 다시 저장하세요. 실제 접속 IP 설정은 서버의 실 IP 설정을 사용합니다.

기본 nginx 설정을 직접 바꿔 사용하는 경우에는 사전 검사를 안전하게 분리할 수 없어 전체 설정 오류로 시작이 중단될 수 있습니다. 검사 후 대상 이름이나 파일 상태가 다시 바뀌어 최종 검사에 실패한 경우나 보존된 설정을 복구하는 중 검사에 실패한 경우도 시작을 중단합니다.


**호스트 폴더 권한.** 시작할 때 DB·앱 폴더와 업그레이드 검사가 확인하는 파일에서 그룹·기타 사용자의 쓰기 권한을 빼고, 비밀 값·인증서 파일과 폴더는 검사가 허용하는 권한으로 맞춥니다. 파일 내용과 소유자는 바꾸지 않으며, 바꾼 경로와 권한은 상태 페이지·`auto-status`·로그에 경고로 표시합니다. 정리된 권한은 1.19.11로 되돌려도 그대로 사용할 수 있습니다. `NWUI_DB_UPGRADE=off`로 이전을 보류해도 2.0.3의 첫 시작에서 이 권한 정리는 진행됩니다. 읽기 전용 연결, root가 아닌 소유자, 권한 변경을 반영하지 않는 공유 폴더처럼 자동으로 정리할 수 없으면 해당 경로·이유·해결 명령을 표시하고 시작을 멈춥니다.

### macOS의 기존 하위 폴더

기존 마운트 루트에 `PG_VERSION`이 있는 배치와 `pgdata/PG_VERSION`에 있는 배치를 알아봅니다. **기존 Compose가 연결하던 부모 폴더를 그대로 유지**하며, 새로 `pgdata`만 연결하거나 데이터를 옮기지 않습니다. 완료 후 PG17 실제 데이터는 같은 DB 루트 안의 `17/data`에 있습니다. `17/data` 자체를 마운트하지 않습니다.

Docker Desktop과 OrbStack의 Mac 공유 폴더에서는 사용자 간 파일 권한 제한이 적용되지 않아 API·웹 화면·nginx 작업 프로세스도 해당 저장소의 DB 파일·암호화 키·비밀 값·설정 게시 파일·로그를 읽고 바꿀 수 있습니다. 이 프로세스 중 하나가 침해되면 컨테이너 안의 root 권한까지 이어질 수 있습니다. 감지된 저장소에 경고를 표시하고 설치·자동 업그레이드를 진행합니다. 컨테이너 내부 프로세스 간 권한 분리가 필요하면 Docker 관리 볼륨을 사용하세요. macOS에서는 가능하면 OrbStack과 Docker 관리 볼륨 사용을 권장합니다. 기존 Docker Desktop·OrbStack Mac 공유 폴더 설치는 같은 DB·앱 폴더를 연결한 채 경고와 함께 업그레이드합니다. 이 경고 때문에 데이터를 옮길 필요는 없습니다. 공유 폴더의 소유자 표시 특성을 반영하며, 파일 형식·접근 모드·잠금·데이터 동기화와 나머지 안전 검사는 유지합니다. `PGDATA`·`PGROOT`를 직접 지정했던 설치가 거부되면 표시된 경로 안내를 따르세요. 원본을 지우거나 빈 볼륨으로 바꾸지 마세요.

## 3. 실패·보류와 재시도

| 표시된 상태 | 다음 조치 |
| --- | --- |
| 업그레이드 전 검사 거부 · 원본 미변경 | 마운트·키·용량·지원하는 DB 구조를 확인. 원인을 고쳐 컨테이너 재시작하거나 기존 이미지 유지 |
| 저장소 권한을 자동으로 정리하지 못함 | 표시된 경로·이유·명령을 확인. 컨테이너를 멈추고 호스트에서 명령을 실행한 뒤 재시작 |
| 재발행 전 2.0.0에서 「저장소 확인」(단계 0/0) `unsupported_layout`으로 멈춤 · 원본 미변경 | 새 2.0.3 이미지를 다시 받아 재시작하면 권한을 정리하고 이어서 진행. 직접 고치려면 아래 「재발행 전 2.0.0에서 저장소 확인에 멈춘 경우」 |
| 원본 PG15 복원 완료 | 같은 시작에서는 다시 이전하지 않고 대기. 아래 다음 조치의 재시도 조건을 따르거나 같은 볼륨으로 1.19.11 이미지로 돌아감 |
| 원본 복원 미완료 | **이미지를 바꾸지 않음.** 표시된 보존 위치를 유지하고 용량·권한 등 원인을 고친 뒤 같은 2.0.3 이미지로 재시작해 복구를 이어감 |
| PG17 전환 후 서비스 시작 실패 | PG17은 유지됨. 재시작해 시작 단계를 이어가거나 아래 명시적 PG15 복원 명령 사용 |
| 업그레이드 보류 | `NWUI_DB_UPGRADE=off`를 제거하고 재시작하거나 기존 1.19.11 이미지 사용. 보류 중 2.0.3 서비스는 실행되지 않음 |
| DB 이전 완료 · 설정 적용 확인 대기 | 서비스 시작 실패가 아님. 적용 상태를 확인할 때까지 이전 데이터 정리를 보류하고, 이후 확인에서 정리를 재시도 |
| 이전 PG15 데이터 정리 보류 · 서비스 정상 | 정상 운영을 유지하며 다음 컨테이너 시작에서 남은 정리를 재개. 별도 보관 PG15 백업은 이 자동 정리 대상이 아님 |

업그레이드가 끝나고 정리만 대기하는 상태는 실패가 아닙니다. 이 상태만으로 서비스 시작 실패 알림을 보내지 않습니다. 정리가 남으면 현재 시작에서 한 번 더 확인할 수 있으며, 남은 작업은 다음 컨테이너 시작에서도 다시 확인합니다. 보관 PG15 백업은 운영자가 제거할 때까지 유지되며, 백업의 존재 자체가 서비스 시작 실패를 뜻하지 않습니다.


### 재발행 전 2.0.0에서 저장소 확인에 멈춘 경우

호스트 폴더 설치에서 DB·앱 폴더나 그 안의 파일에 그룹·기타 사용자의 쓰기 권한이 있으면 다시 발행하기 전의 2.0.0은 「저장소 확인」(단계 0/0)에서 `unsupported_layout`으로 멈춥니다. 원본은 바뀌지 않았습니다. 태그를 `2.0.3`으로 바꿔 이미지를 다시 받아 시작하면 권한을 정리하고 이어서 진행합니다.

이전 2.0.0을 유지한 채 직접 고치려면 컨테이너를 정상 종료하고 「1. 중지·백업·공간 준비」의 백업을 마친 뒤 호스트에서 다음을 실행합니다. `<앱 데이터 호스트 폴더>`는 `/var/lib/nginx-web-ui`에, `<DB 호스트 폴더>`는 `/var/lib/postgresql/data`에 연결한 호스트 폴더입니다.

```sh
sudo chmod go-w '<앱 데이터 호스트 폴더>'
sudo chmod 700 '<앱 데이터 호스트 폴더>/secrets' '<앱 데이터 호스트 폴더>/tls'
sudo find '<앱 데이터 호스트 폴더>/secrets' -maxdepth 1 -type f -exec chmod 600 {} +
sudo chmod 600 '<앱 데이터 호스트 폴더>/tls/default.key'
sudo chmod 644 '<앱 데이터 호스트 폴더>/tls/default.crt'
sudo chmod -R go-w '<DB 호스트 폴더>'
```

`PG_VERSION`이 `<DB 호스트 폴더>` 바로 안에 있으면 `sudo chmod 700 '<DB 호스트 폴더>'`를, `<DB 호스트 폴더>/pgdata` 안에 있으면 `sudo chmod 700 '<DB 호스트 폴더>/pgdata'`를 이어서 실행합니다. 이 권한은 1.19.11에서도 그대로 사용할 수 있습니다.

실패 시 관리 포트에는 상태 안내가 남을 수 있지만 프록시 서비스는 정상 제공되지 않습니다. 상태 페이지도 열리지 않으면 `docker logs --tail=100 nginx-web-ui`를 확인하세요. 권한·설정 저장소 복구 실패에서는 관리 화면 자체가 열리지 않을 수도 있습니다. 오류를 없애려고 키·선택 기록·복구 기록을 지우거나 상태를 수동으로 완료 처리하지 마세요.

자동 이전 실패 후 원본 PG15를 검증 후 복원하면 결과를 표시하고 **같은 시작에서는 다시 이전하지 않고 대기**합니다. 상태 페이지의 다음 조치는 재시도 조건에 따라 다음 중 하나입니다.

- **재시도 가능**: 「원인을 해결한 뒤 컨테이너를 다시 시작하면 자동으로 다시 시도합니다. 같은 볼륨으로 1.19.11 이미지로 되돌릴 수도 있습니다.」
- **대기 시간 있음**: 「원인을 해결한 뒤 표시된 다음 재시도 가능 시각 이후에 컨테이너를 다시 시작하면 자동으로 다시 시도합니다. 같은 볼륨으로 1.19.11 이미지로 되돌릴 수도 있습니다.」
- **같은 원인으로 재시도하지 않음**: 「같은 볼륨으로 1.19.11 이미지로 되돌리세요. 원인을 해결했다면 NWUI_DB_UPGRADE_RETRY에 새 값을 지정하고 다시 시작하면 한 번 더 시도합니다.」

복원 미완료에는 「이미지를 바꾸지 마세요. 공간을 확보하거나 표시된 원인을 해결한 뒤 이 이미지로 컨테이너를 다시 시작하면 복원을 이어서 진행합니다.」라는 안내를 따릅니다.

이전 진단 로그는 `<pgdata volume>/.nwui-auto-work/logs/<attempt>`에 **최신 시도 한 건만** 보관하며 복원 후에도 남습니다. `<pgdata volume>`은 연결한 DB 볼륨, `<attempt>`는 시도 식별값입니다.

다음 두 환경변수는 **PG15 업그레이드가 대기 중일 때만** 의미가 있으며 컨테이너를 시작할 때 한 번 읽습니다.

- `NWUI_DB_UPGRADE=off`는 서비스를 대기 상태로 두고 상태 페이지를 표시합니다. 변수를 제거하고 컨테이너를 재시작하면 이전을 다시 진행할 수 있습니다.
- `NWUI_DB_UPGRADE_RETRY=<식별 문자열>`은 **서로 다른 값마다 정확히 한 번** 재시도를 요청합니다. 같은 값을 다시 넣어도 그 값으로 추가 재시도를 요청하지 않습니다. 값은 비어 있지 않은 128자 이하 문자열이고 NUL 문자가 없어야 합니다. 새 값을 지정하기 전에 원인을 먼저 고치세요.

## 4. 1.19.11로 되돌리기

**PG17 파일을 PG15로 낮출 수는 없습니다. 성공한 업그레이드 뒤에는 이미지 태그만 1.19.11로 낮추지 마세요.** 보관된 PG15를 명시적으로 복원해야 하며 업그레이드 이후 사용자 변경·예약 작업·통계 등 **모든 쓰기를 잃습니다.** 독립 백업과 기존 실행 설정도 계속 보관하세요.

상태가 ‘원본 미변경’ 또는 ‘PG15 복원 완료’라면 추가 복원 없이 기존 이미지로 돌아갈 수 있습니다. **‘복원 미완료’에는 아래 명령을 대신 실행하지 말고 먼저 같은 이미지에서 복구를 끝냅니다.**

성공 후 되돌리기는 다음 순서입니다. `auto-status`가 보여 준 실제 백업 ID를 사용합니다.

```sh
docker exec nginx-web-ui nwui-db-upgrade auto-status
docker compose -f compose.yaml stop -t 600

NWUI_UPGRADE_IMAGE=ghcr.io/xelexis-dev/nginx-web-ui:2.0.3
NWUI_BACKUP_ID=REPLACE_WITH_STATUS_BACKUP_ID

nwui_upgrade() {
  docker run --rm --pull=never --security-opt no-new-privileges:true --network none \
    --volumes-from nginx-web-ui \
    --entrypoint /usr/local/bin/nwui-db-upgrade \
    "$NWUI_UPGRADE_IMAGE" "$@"
}

nwui_upgrade restore-pg15 --confirm-loss "$NWUI_BACKUP_ID"
```

기존 이미지가 ARM64라면 변수의 태그도 `2.0.3-arm64`로 바꿉니다. 이 예시는 **중지된 기존 컨테이너가 남아 있는 기본 설치**이며 `--volumes-from`으로 그 DB·앱 마운트를 그대로 연결합니다. `restore-pg15`는 데이터 암호화 키를 읽습니다. 기본 키는 앱 볼륨에 있어 위 마운트로 연결되지만, 사용자 지정 `DATA_KEY_FILE`을 쓴다면 함수의 `docker run`에 `-e DATA_KEY_FILE=<기존과 같은 절대 경로>`를 추가하고 해당 키 파일의 마운트도 유지하세요. `--volumes-from`은 환경변수를 복사하지 않습니다. `auto-status`·`remove-pg15-backup`·`remove-discarded`는 키를 읽지 않으므로 이 설정이 필요하지 않습니다. 같은 볼륨을 쓰는 다른 컨테이너도 모두 중지돼 있어야 합니다.


복원·정리 명령은 **종료 코드 0이면 성공, 그 밖에는 실패**입니다. 메시지와 `auto-status`를 함께 확인하세요. PG15 복원 완료가 확인된 뒤에만 Compose 이미지 주소를 `1.19.11`(AMD64) 또는 `1.19.11-arm64`(ARM64)로 바꾸고 시작합니다. digest로 고정했다면 태그만 바꾸지 말고 보관한 1.19.11의 전체 이미지 주소로 교체하세요. 2.0.3의 digest가 남으면 2.0.3가 다시 실행됩니다.

```sh
docker compose -f compose.yaml pull
docker compose -f compose.yaml up -d
docker logs --tail=100 nginx-web-ui
```

되돌리면서 분리 보관한 PG17 데이터는 자동 삭제하지 않습니다. 다시 2.0.3으로 갈 때는 PG15를 기준으로 새 자동 이전을 진행합니다. 이전 PG17의 변경 내용이 자동 합쳐지는 것은 아닙니다.

`restore-pg15`의 완료 메시지와 이후 상태 안내도 재시도 조건을 구분합니다.

- **재시도 가능**: 「PG15로 복원했습니다. 1.19.11로 시작하거나, 2.0 이미지를 다시 시작하면 새 이전을 진행합니다.」
- **대기 시간 있음**: 「PG15로 복원했습니다. 1.19.11로 시작하거나, 표시된 다음 재시도 가능 시각 이후 2.0 이미지를 다시 시작하면 새 이전을 진행합니다.」
- **같은 원인으로 재시도하지 않음**: 「PG15로 복원했습니다. 1.19.11로 시작하세요. 이 이미지는 같은 원인으로 다시 이전하지 않습니다. 원인을 해결했다면 NWUI_DB_UPGRADE_RETRY에 새 값을 지정하고 다시 시작하세요.」

## 5. 보관 백업·분리 보관 데이터 정리

### PG15 백업 제거

2.0.3가 정상 시작했고 복원 가능한 독립 백업을 확보했으며 PG15로 돌아갈 필요가 없다고 판단한 뒤에만 정리합니다. 삭제는 자동으로 하지 않습니다. **보관 PG15를 지우면 그 백업을 이용한 `restore-pg15`는 불가능합니다.**

**`restore-pg15`를 끝낸 뒤에는 백업이 현재 PG15 데이터로 이동해 소비됐으므로 `remove-pg15-backup`을 사용할 수 없습니다. 지울 백업이 남아 있지 않습니다.** 이때 남는 분리 보관 PG17은 다음 절의 명령으로 정리합니다.

먼저 `auto-status`가 표시하는 실제 백업 ID를 확인하고 서비스를 중지합니다. 위 `nwui_upgrade` 함수와 같은 2.0.3 이미지·DB·앱 마운트로 실행합니다.

```sh
docker exec nginx-web-ui nwui-db-upgrade auto-status
docker compose -f compose.yaml stop -t 600
nwui_upgrade remove-pg15-backup --confirm "$NWUI_BACKUP_ID"
docker compose -f compose.yaml start
```

### PG15 복원 후 분리된 PG17 제거

`restore-pg15`가 만들어 분리 보관한 PG17도 더 이상 필요 없으면 제거할 수 있습니다. **해당 복원이 만든 분리 보관 대상**이어야 하며 ID는 상태의 `discarded[].id` 값을 사용합니다. 백업 ID와 같다고 가정하지 마세요. 진단 로그는 이 분리 보관 대상과 별도로 최신 시도 한 건을 보존합니다. PG15로 돌아간 뒤라도 모든 서비스를 중지하고 같은 DB·앱 마운트에 2.0.3 이미지의 오프라인 명령을 사용합니다.

다음은 위 `nwui_upgrade` 함수가 정의돼 있고 서비스를 중지한 상태의 예시입니다. 실제 ID와 상태를 먼저 확인하고 실행합니다.

```sh
nwui_upgrade auto-status
NWUI_DISCARDED_ID=REPLACE_WITH_STATUS_DISCARDED_ID
nwui_upgrade remove-discarded --confirm "$NWUI_DISCARDED_ID"
```

지정한 대상 한 개만 삭제하며 결과와 확보한 공간을 표시합니다. **종료 코드 0이면 성공, 그 밖에는 실패**입니다. 메시지와 `auto-status`를 확인하세요. 조건을 충족하지 못한 명령을 성공으로 처리하거나 직접 폴더를 지우지 않습니다. 작업을 마친 뒤 선택한 서비스 이미지를 다시 시작하세요.

## 로그인·노드·기존 프록시 확인

- 2.x 안에서는 **게이트웨이를 먼저, 노드를 나중에** 업데이트합니다. 노드는 같은 메이저에서 자기보다 새 minor의 게이트웨이를 받아들이지만, 게이트웨이는 자기보다 새 minor의 노드를 관리하지 않습니다. 1.19 노드 연결은 2.0.x 게이트웨이에서 지원합니다.
- 2.0과 1.19 노드는 양방향 연결을 지원하며 동시에 올릴 필요는 없습니다. 이전 노드에서는 실행 중 이름 확인·새 재시도 등 새 기능을 사용할 수 없습니다. 2.0 전용 설정을 쓰는 항목은 1.19 관리 화면의 저장을 거부해 보호하므로 해당 관리 화면을 업데이트하세요.
- 더 새 버전으로 업데이트한 DB를 이전 버전 이미지로 시작하면, DB 구조가 바뀐 경우 시작하지 않고 `database_newer`를 표시합니다. 데이터는 바꾸지 않습니다. 이전 버전으로 돌아가려면 업데이트 전에 만든 DB·앱 백업 쌍을 복원하세요. DB 구조가 같은 버전 사이의 되돌림은 이 검사로 막지 못합니다.
- 2.0 노드 주소는 같은 노드 확인 후 바꿀 수 있습니다. 오래된 연결은 원래 주소에서 상태를 새로고침해야 할 수 있습니다. 1.19 노드나 다른 노드로 확인된 주소는 새 토큰으로 다시 연결해야 합니다.
- 기존 서버 토큰은 기존 동작을 유지합니다. 새 토큰은 필요한 관리 권한과 만료 시간을 선택합니다. 로그인은 업데이트 후 첫 세션 갱신부터 최대 30일로 제한되며, 2단계 인증 변경은 다른 세션을 종료합니다.
- 관리 포트와 내부 서비스 포트는 기존처럼 예약됩니다. 같은 컨테이너의 내부 웹·API·DB 서비스로 우회 연결하는 설정과 메타데이터 주소 등은 거부될 수 있습니다. 다른 컨테이너의 `grafana:3000`·`app:8080` 같은 대상은 사용할 수 있습니다.
- 인증서가 없거나 발급 완료 자료가 없는 서버는 적용에서 제외하고 알립니다. 인증서 발급·교체·갱신만으로 저장된 다른 편집 내용을 공개하지 않습니다. 새 인증서로 보류된 서버를 켜거나 현재 설정의 인증서 참조를 정리할 때는 위쪽 막대의 「적용」으로 설정을 한 번 적용하세요.
- 정상적인 기존 설정은 계속 저장·적용할 수 있습니다. 적용 보류·유지·순환 가능성 경고는 실제 응답과 함께 확인하세요. nginx 실행파일·기본 설정·추가 포함 파일은 관리 요청 처리 사용자가 소유하거나 쓸 수 있으면 거부됩니다. root 소유는 필수 조건이 아닙니다. 오류가 지정한 파일·권한만 수정하고 전체 데이터의 권한을 넓히지 마세요.

기존 사용자 지정 `DATA_KEY_FILE`은 파일 모드 자체로 거부하지 않지만 관리 요청 처리 사용자가 소유하거나 쓸 수 없어야 합니다. 누구나 쓸 수 있는 상위 폴더는 sticky bit 보호가 필요합니다. 키가 없는 설치에서 읽기 전용 사용자 지정 경로를 쓰면 원본을 바꾸기 전에 거부합니다. Docker Desktop·OrbStack의 macOS 호스트 폴더 환경에서는 사용자 지정 `DATA_KEY_FILE`을 앱 저장소 안(기본 경로 또는 `/var/lib/nginx-web-ui` 아래)이나 Docker 관리 볼륨에 두어야 합니다. 그 밖의 위치에 따로 연결한 호스트 폴더의 키 파일은 접근 제한을 확인할 수 없어 거부합니다. 2.0 시작에서는 `pair_mismatch`, 1.19에서 업그레이드할 때는 `key_path_invalid`로 표시되며 데이터는 바꾸지 않습니다.

이 안내는 **2.0.3** 기준입니다. 이후 버전은 해당 버전의 업그레이드 안내를 따르세요.
