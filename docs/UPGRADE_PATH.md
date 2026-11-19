# 업그레이드 경로

대상 버전: **2.1.2**

**1.19.x → 2.0.3 → 2.1.2** 순서로 올립니다. 2.0.3이 1.x의 필수 경유 버전이며 마지막 2.0.x입니다. 2.1.0 이후에는 2.0.x 릴리스를 더 내지 않지만, 이전에 필요한 2.0.x 이미지는 계속 제공합니다. 1.19.11이 마지막 1.x이며 이후 1.x 릴리스도 없습니다. `latest`는 1.19.11에 남고 메이저 버전을 넘어 이동하지 않으므로 버전 태그를 지정하세요.

2.1.2는 PostgreSQL 17(PG17)을 사용하고 PostgreSQL 15(PG15) 이전·복원 도구는 포함하지 않습니다. **1.x 데이터나 이전을 마치지 못한 상태에서는 DB·앱 데이터를 변경하지 않고 시작을 거부합니다.** 2.0.x에서 이전을 마친 설치는 2.1.2로 직접 업데이트할 수 있습니다. 2.1.0·2.1.1에서도 중간 버전 없이 올릴 수 있으며 DB 구조를 바꾸지 않습니다. 2.0.x에서 처음 2.1로 올릴 때는 앱의 DB 구조를 갱신하며, 2.1 이전 사용자 API 토큰은 폐기됩니다. 필요한 사용자 토큰을 다시 발급하세요. 서버·게이트웨이 토큰은 유지합니다.

변경 내용은 [업데이트 내역](https://github.com/xelexis-dev/nginx-web-ui/blob/main/docs/UPDATE.md), 설정 파일 전문과 새 설치는 [설치 안내](https://github.com/xelexis-dev/nginx-web-ui/blob/main/docs/README.ko.md)를 확인하세요.

## 지원 경로

| 현재 설치 | 2.1.2로 가는 방법 | 필수 중간 버전 |
| --- | --- | --- |
| 1.19.0–1.19.11 · PG15 | [2.0.3 안내](https://github.com/xelexis-dev/nginx-web-ui/blob/v2.0.3/docs/README.ko.md)대로 같은 DB·앱 볼륨으로 이전을 완료하고, 로그인·프록시 응답을 확인한 뒤 2.1.2로 교체 | **2.0.3** |
| 1.18.x · PG15 | 1.19.11 경유를 권장. 그다음 2.0.3에서 이전 완료 후 2.1.2로 교체. 1.18.x 직접 이전의 검증 범위는 2.0.3 안내를 확인 | **2.0.3**, 1.19.11 권장 |
| 1.18.0 미만 | 먼저 1.19.11로 일반 업데이트한 뒤 2.0.3에서 이전하고 2.1.2로 교체 | **1.19.11 → 2.0.3** |
| 2.1.0·2.1.1 | DB·앱 백업 쌍을 보관하고 같은 볼륨·아키텍처로 2.1.2 교체. DB 구조 변경 없음 | 없음 |
| 2.0.x · PG17 · 이전 완료 | DB·앱 백업 쌍을 보관하고 같은 볼륨·아키텍처로 2.1.2 교체 | 없음 |
| 2.0.x에서 이전·복원이 미완료되거나 보류된 상태 | 2.1.2로 건너뛰지 말고 2.0.3에서 표시된 복구·재시도를 마친 뒤 진행 | **2.0.3에서 완료** |
| Linux 커널 5.11 미만 또는 필요한 시스템 호출이 차단된 환경 | 지원하는 커널과 Docker 엔진을 먼저 준비. 시작 거부의 안내 코드를 확인 | 지원 환경 필요 |
| 사용자가 바꾼 DB 구조 등 지원하지 않는 상태 | 거부 이유를 확인하고 원본·백업 보존. 데이터 초기화나 버전 파일 변경으로 우회하지 않음 | 지원 조건 확인 |

재발행 전 2.0.0·2.0.1을 사용했던 PG17 설치도 DB·앱을 함께 백업한 뒤 2.1.2로 올립니다. 오래된 이미지에 없던 DB 구조를 추가할 수 있으므로, 되돌릴 때는 업데이트 전에 보관한 백업 쌍이 필요합니다. 1.x 원본이나 저장소 확인에 멈춘 설치는 위 2.0.3 경유 조건을 먼저 충족해야 합니다.

## 1. 중지·백업 준비

- 기존 이미지·Compose·환경 설정, DB와 인증·암호화 키가 있는 앱 데이터를 **같은 시점의 쌍**으로 백업하고 복원 가능 여부를 확인합니다. 따로 지정한 `DATA_KEY_FILE`과 그 마운트도 보존하세요. 보관 PG15 백업은 독립 백업을 대신하지 않습니다.
- 컨테이너를 정상 종료하고 로그의 종료 결과를 확인하세요. 중지에는 최대 600초를 허용하며 강제 종료하지 않습니다. 자동 재시작 도구가 중지·백업·업데이트를 방해하지 않게 합니다.
- 같은 DB·앱 볼륨을 두 컨테이너가 동시에 쓰지 않게 하세요. 비교·복원 확인에는 독립된 사본을 사용합니다.
- 백업과 DB 구조 갱신에 필요한 공간·inode를 확보합니다. macOS에서는 Docker의 디스크 한도도 확인하세요. PG15 변환에 필요한 공간과 실패·재시도는 [2.0.3 업그레이드 안내](https://github.com/xelexis-dev/nginx-web-ui/blob/v2.0.3/docs/UPGRADE_PATH.md)를 따릅니다.
- Linux에는 커널 5.11 이상과 최신 Docker 엔진이 필요합니다. 호스트 로그 폴더는 nginx 로그 root:nginx·0750, 앱 로그 root·0755로 정리하므로 외부 로그 수집기의 접근 권한도 확인합니다.

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

정상 종료와 DB 종료 로그를 확인한 뒤 다음으로 진행합니다. 종료 코드가 0이 아니거나 강제 종료됐다면 원인을 확인하세요. 다시 발행하기 전의 2.0.0 이미지를 중지할 때는 정상 종료여도 `Error: interrupted: 시작 준비를 중단했습니다…` 한 줄이 로그에 남을 수 있습니다. 종료 코드가 0이고 DB 종료 로그(`database system is shut down`)가 있으면 정상 종료입니다. 2.1.2은 이 오류 줄 대신 `[upgrade] 중지 요청을 받아 종료합니다.` 한 줄만 남깁니다. 같은 DB·앱 저장소에 쓰는 다른 컨테이너도 모두 중지하고, 백업이 끝날 때까지 다시 시작하지 않습니다.

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

## 2. 같은 데이터로 2.1.2 이미지 교체

**기존 컨테이너 이미지의 아키텍처를 유지**합니다. AMD64는 `2.1.2`, ARM64는 `2.1.2-arm64`입니다. 호스트 아키텍처만 보고 DB 파일을 다른 아키텍처로 옮기지 마세요. 같은 볼륨명·마운트·포트·인증 설정을 유지하고, digest로 고정했다면 이미지 주소 전체를 교체합니다. 빈 볼륨을 새로 만들지 않습니다.

```sh
docker image inspect "$(docker inspect nginx-web-ui --format '{{.Image}}')" --format '{{.Architecture}}'
docker compose -f compose.yaml stop -t 600
docker logs --tail=100 nginx-web-ui
# 정상 종료 후 위 절차로 DB·앱과 외부 키를 함께 백업하고 이미지 태그를 변경합니다.
docker compose -f compose.yaml config --quiet
docker compose -f compose.yaml pull
docker compose -f compose.yaml up -d
docker logs --tail=100 nginx-web-ui
docker exec nginx-web-ui nwui-db-upgrade auto-status
```

기본 관리 주소는 `http://<Docker 서버 주소>:8080`입니다. 시작 중에는 같은 컨테이너의 80/443을 거치는 도메인 대신 이 호스트 관리 포트와 로그에서 상태를 확인합니다. 실행 중이라는 표시만으로 완료를 판단하지 말고 계정 메뉴의 버전 2.1.2, 기존 계정 로그인과 실제 프록시 응답을 확인하세요. 최초 관리자 생성 화면이 나타나면 계정을 새로 만들지 말고 마운트를 확인합니다.

### 노드 순서와 API 토큰

2.x 안에서는 게이트웨이를 먼저 올린 뒤 노드를 올립니다. 같은 메이저에서는 노드가 자기 minor 이상의 게이트웨이를 받지만, 게이트웨이는 자기보다 새 minor의 노드를 관리하지 않습니다. 2.1 게이트웨이는 2.0 노드를 관리할 수 있으나 HTTP/2를 켜려면 노드도 2.1 이상이어야 합니다.

2.0·1.19의 혼용 예외는 2.1에 적용되지 않습니다. 1.19 노드가 있다면 게이트웨이를 2.0.3으로 올리고 노드도 2.0.3에서 이전을 마친 뒤, 게이트웨이 2.1.2 → 노드 2.1.2 순서로 진행합니다.

2.1 이전 사용자 토큰은 폐기되며, 새 사용자 토큰의 발급·사용에는 계정의 2단계 인증 등록이 필요합니다. 발급 전에는 이번 로그인의 인증 확인도 마쳐야 합니다. 서버·게이트웨이 연결 토큰은 바꾸지 않습니다. 자동화에 필요한 사용자 토큰을 다시 발급하고 HTTPS로 호출해 확인하세요.

## 3. 시작 거부와 복구

1.x 데이터나 미완료 이전 상태에는 `requires_2_0_first`와 다음 안내가 표시됩니다.

> 1.x(PostgreSQL 15) 데이터입니다. 먼저 2.0.x 최신(2.0.3 이상) 이미지에서 이전을 완료한 뒤 2.1로 올리세요. 기존 데이터는 변경하지 않았습니다.

2.1.2 컨테이너를 중지하고 DB·앱 볼륨을 유지한 채 **2.0.3 안내에 따라 이전·복구를 먼저 마칩니다.** 2.1의 `NWUI_DB_UPGRADE=off`는 무시되며 이 거부를 해제하지 못합니다. 2.1의 옛 이전·PG15 복원 명령도 안내만 표시하고 실행을 거부합니다.

이미 2.1에서 사용한 PG17 데이터를 2.0 이미지로 정상 시작하면 `database_newer`로 거부됩니다. 이전 버전으로 되돌리려면 업데이트 전 DB·앱 백업 쌍을 복원하세요. **PG15 복원을 위한 2.0.3 오프라인 도구 실행과 2.0의 정상 시작은 다릅니다.** 기록이나 DB 버전 파일을 지워 더 오래된 이미지가 시작하게 만들지 마세요.

다른 거부 코드는 [설치 문제 해결](https://github.com/xelexis-dev/nginx-web-ui/blob/main/docs/README.ko.md#7-설치-문제-해결)과 로그를 확인하세요. 마운트·키 쌍을 보존하고 표시된 원인을 해결한 뒤 같은 이미지로 시작을 다시 시도합니다.

## 4. 1.19.11로 되돌리기

2.1 사용 후 보관된 PG15 백업으로 돌아갈 때는 **컨테이너를 중지하고 DB·앱 볼륨을 둘 다 보존한 뒤 2.0.3 이미지의 오프라인 진입점**에서 `restore-pg15 --confirm-loss <backupId>`를 실행합니다. **변환 이후 모든 변경을 잃습니다.** 복원이 성공한 뒤에만 같은 아키텍처의 1.19.11로 시작하세요. 보관 PG15 백업이 없으면 이 방법을 쓸 수 없으며, 독립된 업데이트 전 DB·앱 백업 쌍을 복원해야 합니다. 2.0을 정상 시작해 2.1 데이터를 열면 안 됩니다.

다음은 중지된 기존 컨테이너가 남아 있고 기본 저장 경로를 쓰는 예시입니다. ARM64는 복원 이미지의 태그를 `2.0.3-arm64`로 바꿉니다. `--volumes-from`은 마운트만 복사하므로 사용자 지정 저장 경로 환경변수를 명시적으로 전달하세요. 사용자 지정 `DATA_KEY_FILE`이 있으면 **같은 키 파일 마운트와 `-e DATA_KEY_FILE=<기존 절대 경로>`**도 `docker run`에 추가합니다. 기본 키는 앱 볼륨에 있습니다.

```sh
docker exec nginx-web-ui nwui-db-upgrade auto-status
docker compose -f compose.yaml stop -t 600
docker logs --tail=100 nginx-web-ui
NWUI_RESTORE_IMAGE=ghcr.io/xelexis-dev/nginx-web-ui:2.0.3
NWUI_BACKUP_ID=REPLACE_WITH_STATUS_BACKUP_ID
docker pull "$NWUI_RESTORE_IMAGE"
docker run --rm --pull=never --security-opt no-new-privileges:true --network none \
  --volumes-from nginx-web-ui \
  --entrypoint /usr/local/bin/nwui-db-upgrade \
  "$NWUI_RESTORE_IMAGE" restore-pg15 --confirm-loss "$NWUI_BACKUP_ID"
```

복원이 성공한 뒤 종료 코드 0과 완료 안내를 확인하고, Compose 이미지를 같은 아키텍처의 `1.19.11` 또는 `1.19.11-arm64`로 바꿔 `pull`·`up -d`로 시작합니다. digest로 고정했다면 보관한 1.19.11의 전체 이미지 주소로 교체하세요. 복원·재시도·분리 보관 데이터의 조건은 [2.0.3 업그레이드 안내](https://github.com/xelexis-dev/nginx-web-ui/blob/v2.0.3/docs/UPGRADE_PATH.md)를 따릅니다. 복원한 데이터로 2.1을 곧바로 시작하면 다시 2.0.3 경유를 요구합니다.

## 5. 보관 백업·분리 보관 데이터 정리

2.0에서 이전을 마친 설치는 PG15 백업과 「PG15 백업을 보관하고 있습니다」 안내가 유지됩니다. 2.1에서도 `auto-status`, `remove-pg15-backup`, `remove-discarded`를 사용할 수 있습니다. 정상 운영과 독립 백업을 확인하고 PG15로 돌아갈 필요가 없다고 판단한 뒤에만 삭제하세요. **삭제 후에는 그 백업으로 복원할 수 없습니다.** 독립 백업을 지우는 명령은 아닙니다.

다음은 **복원과 별개인 정리** 예시입니다. `auto-status`의 실제 ID를 사용하며, ARM64는 `2.1.2-arm64`로 바꾸고 사용자 지정 저장 경로 환경변수도 기존과 맞춥니다.

```sh
docker exec nginx-web-ui nwui-db-upgrade auto-status
docker compose -f compose.yaml stop -t 600
NWUI_MAINT_IMAGE=ghcr.io/xelexis-dev/nginx-web-ui:2.1.2
NWUI_BACKUP_ID=REPLACE_WITH_STATUS_BACKUP_ID
docker run --rm --pull=never --security-opt no-new-privileges:true --network none \
  --volumes-from nginx-web-ui \
  --entrypoint /usr/local/bin/nwui-db-upgrade \
  "$NWUI_MAINT_IMAGE" remove-pg15-backup --confirm "$NWUI_BACKUP_ID"
docker compose -f compose.yaml start
```

PG15 복원을 마치면 그 백업은 현재 데이터로 이동해 소비되므로 더는 `remove-pg15-backup`으로 지울 수 없습니다. 복원 과정에서 분리 보관된 PG17만 정리하려면, 서비스를 중지한 상태에서 위 오프라인 명령의 끝을 `remove-discarded --confirm <id>`로 바꾸고 `auto-status`의 `discarded[].id`를 사용합니다. 정리 명령은 종료 코드 0이면 성공이며 실패하면 안내를 확인하세요.
