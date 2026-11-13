# fail2ban 연동

호스트의 fail2ban 이 nginx-web-ui 의 로그를 읽고 밴하도록 붙이는 방법.
**이 파일들은 컨테이너 안에서 동작하지 않는다** — 이미지에는 참고용으로만 들어 있고
(`/usr/share/nginx-web-ui/fail2ban/`), 실제로는 호스트에 복사해서 쓴다.

## 왜 앱 전용 로그가 따로 있나

로그인 **성공과 실패가 nginx access.log 에서 구분되지 않는다.** 둘 다 `POST /login → 303` 이고
차이는 `Location` 헤더뿐인데, `combined`/`combined_host` 포맷은 이를 기록하지 않는다
(2026-08-09 실측). 서버 액션이라 실패가 401 을 내지도 않는다.

그래서 앱이 `/var/log/nginx-web-ui/auth.log` 에 fail2ban 이 파싱할 수 있는 한 줄을 직접 쓴다:

```
2026-08-09T14:05:00.123Z nginx-web-ui auth-failure ip=203.0.113.5 reason=bad_credentials
```

정리하면 로그는 두 갈래이고 역할이 다르다:

| 로그 | 경로 | 잡는 것 |
|---|---|---|
| 앱 인증 실패 | `/var/log/nginx-web-ui/auth.log` | 로그인 브루트포스 |
| nginx access/error | `/var/log/nginx/*.log` | 스캐너·봇·4xx 폭주 |

## 설치

**1. 로그 디렉토리를 호스트로 마운트한다**

```bash
docker run ... \
  -v /srv/nginx-web-ui/log/nginx:/var/log/nginx \
  -v /srv/nginx-web-ui/log/app:/var/log/nginx-web-ui \
  ...
```

compose 를 쓰면 `docker-compose.yml` 의 `nginx-log` / `app-log` 볼륨을 호스트 경로로 바꾼다.

**2. 파일을 꺼내 호스트 fail2ban 에 넣는다**

```bash
docker cp nginx-web-ui:/usr/share/nginx-web-ui/fail2ban/filter.d/nginx-web-ui-auth.conf \
  /etc/fail2ban/filter.d/
docker cp nginx-web-ui:/usr/share/nginx-web-ui/fail2ban/jail.d/nginx-web-ui.conf \
  /etc/fail2ban/jail.d/
```

**3. `jail.d/nginx-web-ui.conf` 의 `logpath` 를 실제 마운트 경로로 고친다**
(위 예시라면 `/srv/nginx-web-ui/log/app/auth.log`, `/srv/nginx-web-ui/log/nginx/error.log`)

**4. filter 를 실측한다 — 넘어가지 말 것**

```bash
fail2ban-regex /srv/nginx-web-ui/log/app/auth.log \
  /etc/fail2ban/filter.d/nginx-web-ui-auth.conf
```

`Lines: N lines, 0 ignored, N matched, 0 missed` 가 나와야 한다.
`0 matched` 면 밴은 영원히 걸리지 않으면서 설정은 정상으로 보인다.

> 이 저장소의 filter/jail 은 **살아 있는 fail2ban 으로 검증되지 않았다**(개발 환경에
> fail2ban 이 없다). 로그 줄 형식은 실측했으므로, 위 `fail2ban-regex` 로 한 번만
> 확인하면 된다.

**5. 적용**

```bash
fail2ban-client reload
fail2ban-client status nginx-web-ui-auth
```

## ★ 반드시 확인할 것 — DOCKER-USER 체인

Docker 로 publish 한 포트(80/443/8080)로 오는 트래픽은 **DNAT 을 거쳐 FORWARD 로 흐른다.**
fail2ban 기본 action 은 `INPUT` 에 규칙을 넣으므로, 밴 목록에는 IP 가 올라가는데
**실제로는 하나도 막히지 않는다.** `chain = DOCKER-USER` 를 반드시 지정해야 한다
(동봉한 jail 예시에는 들어 있다).

검증: 밴이 걸린 뒤 `iptables -L DOCKER-USER -n` 에 해당 IP 의 DROP 규칙이 보이는지 확인한다.

## 로그 로테이션

로그 디렉토리를 볼륨으로 노출하면 **로그가 컨테이너 재생성 후에도 계속 누적된다**
(그 전에는 컨테이너와 함께 사라졌다). 호스트 logrotate 를 걸 것:

```
/srv/nginx-web-ui/log/nginx/*.log {
    daily
    rotate 14
    compress
    missingok
    notifempty
    postrotate
        docker exec nginx-web-ui nginx -s reopen
    endscript
}

/srv/nginx-web-ui/log/app/auth.log {
    weekly
    rotate 8
    compress
    missingok
    notifempty
    copytruncate
}
```

앱 로그는 `copytruncate` 를 쓴다 — 앱이 파일 핸들을 계속 열어두지 않고 매 기록마다
append 하지만, 재시작 신호 경로가 없으므로 이쪽이 안전하다.

## 엣지(Cloudflare 등) 뒤에 있다면

밴 대상이 엣지 IP 가 되지 않도록, 서버별 **"엣지 / 프록시 뒤 배치 (실 IP 복원)"** 설정을
먼저 켜야 한다. 그래야 `access.log` 와 앱 로그 양쪽에 실제 방문자 IP 가 남는다.

관리 UI(:8080) 자체를 외부 프록시 뒤에 두었다면, 앱이 기록하는 IP 는 그 프록시의 IP 다
— 이 경우 프록시가 `X-Real-IP` 를 실제 클라이언트 IP 로 설정하도록 해야 한다.

## 한계

- 밴은 **호스트 방화벽**이 집행한다. 이 앱은 관여하지 않는다.
- 앱 로그는 계정 열거를 돕지 않도록 **이메일을 남기지 않는다.** 어떤 계정이 공격받는지는
  관리 UI 의 감사 로그에서 `auth:login-locked` 항목으로 확인한다(계정이 잠긴 순간 1건 기록).
