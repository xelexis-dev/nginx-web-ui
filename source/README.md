# nginx-web-ui

A web control panel for nginx: manage L7 reverse-proxy servers and L4 stream
(TCP/UDP) proxies from a browser instead of hand-editing config files on the
server. Includes auth + role-based access control (admin/editor/viewer),
optional TOTP two-factor auth, a full audit log of every
change, live tailing of nginx access/error logs, live nginx metrics on the
dashboard, config history with a per-snapshot diff and one-click rollback
to any previous applied snapshot, and **automatic HTTPS**: issue Let's Encrypt
certificates from the UI (ACME HTTP-01), link them to reverse-proxy servers
for `443 ssl` serving with an optional HTTP→HTTPS redirect, with automatic
renewal ~30 days before expiry and issuance/renewal failures surfaced in the
web UI.

버전별 변경 사항은 [`UPDATE.md`](https://github.com/xelexis-dev/nginx-web-ui/blob/main/docs/UPDATE.md)에 정리되어 있습니다.

### Security

- **Passwords** must be at least 8 characters and mix letters and digits
  (enforced on create, reset, and self-change).
- **Two-factor auth (TOTP):** any user can enable it under **계정 → 2FA**
  (scan a QR, confirm a code, save the one-time backup codes). 2FA is
  **optional by default**; administrators may configure requirements. An admin can
  **require** it per role (admin accounts / regular accounts) under the admin
  panel **보안 (2FA)** (`/admin/security`); users of a required role are then
  confined to the enrollment page until they enable it. Toggling a role's
  requirement forces those users to re-login so it takes effect immediately.
  At login, 2FA-enabled accounts must supply a 6-digit code or a single-use
  backup code in addition to their password.
- **Login throttling** is applied on two independent 15-minute windows: 10
  failures per account, and 30 per client IP across every account it tries.
  The per-IP window is what bounds an attacker who rotates the email on every
  attempt — the per-account cap alone never trips for one.
- Every change (config edits, applies, logins/logouts, user + 2FA management)
  is written to the audit log, along with each lockout (`auth:login-locked`,
  `auth:ip-locked`). Individual failed attempts go to a file instead — see
  [fail2ban](#blocking-abusive-clients-fail2ban) below.

## Architecture

Everything ships in a **single Docker image**: Rust API + Next.js SSR + nginx + PostgreSQL, supervised by s6-overlay.

- **nginx** exposes the admin interface on :8080, routing /api/ and ACME challenges to the loopback Rust API (:3001) and pages to Next (:3000). It also serves managed L7/L4 configurations.
- **Rust API** owns authentication, authorization, PostgreSQL, encrypted secrets, nginx apply/rollback, certificate jobs, remote nodes, metrics, notifications and SSE. Apply operations have one owner; certificate and notification queues persist recovery state.
- **Next.js** renders pages and browser forms using the HTTP API. Its process receives no database URL, authentication secret or encryption key.
- **PostgreSQL** runs inside the same container. Existing data and key volumes remain unchanged.
- **s6-overlay** initializes secrets and PostgreSQL, runs the migration/configuration preparation, starts nginx, waits for API readiness, then starts Next.

Source layout:
- `apps/api` — Rust/axum API and integration tests.
- `apps/web` — Next SSR and browser UI.
- `packages/contracts` — canonical OpenAPI and generated TypeScript response types.
- `packages/presentation`, `packages/ui` — pure presentation helpers and components.
- `packages/db/drizzle` — append-only SQL migration history consumed by Rust.

## Run it (Docker)

Published image tags are architecture-specific:

| Platform | Tag |
| --- | --- |
| Linux AMD64 | `<version>` (also `sha-<40-character-commit>-amd64`) |
| Linux ARM64 | `<version>-arm64` (also `sha-<40-character-commit>-arm64`) |

For ARM64, use `ghcr.io/xelexis-dev/nginx-web-ui:<version>-arm64` with the same
ports, environment variables and volumes. The unsuffixed tags remain AMD64;
they do not automatically select an ARM64 image.

With Docker Compose (recommended):

```sh
docker compose up -d --build
```

This builds the image and publishes 80 (HTTP/ACME), 443 (HTTPS) and 8080
(admin UI), with data in the `nwu-pg`/`nwu-app` named volumes. See
[`docker-compose.yml`](./docker-compose.yml) for the environment-variable
examples and how to publish additional per-server ports;
`docker-compose.dev.yml` is a local-development override (admin UI on :20080).

Or with plain `docker run`:

```sh
docker build -t nginx-web-ui .
docker run -d -p 8080:8080 -p 80:80 -p 443:443 \
  -v nwu-pg:/var/lib/postgresql/data \
  -v nwu-app:/var/lib/nginx-web-ui \
  -v nwu-nginx-log:/var/log/nginx \
  -v nwu-app-log:/var/log/nginx-web-ui \
  nginx-web-ui
```

- **Admin UI:** http://localhost:8080 (nginx `:8080` -> Rust API `:3001`, Next SSR `:3000` inside the container).
- **First admin (web setup):** on first boot with an empty database, open the
  Admin UI — any URL redirects to a one-time `/setup` screen where you create
  the first admin (email + password). Once created, `/setup` is no longer
  reachable. No admin password is generated or printed.
  To provision the admin non-interactively instead, pass env vars at
  `docker run` time: `-e ADMIN_EMAIL=... -e ADMIN_PASSWORD=...` (with these set,
  the admin is seeded on first boot and `/setup` is skipped).
- **User-defined proxy/stream ports:** servers you create in the UI (reverse
  proxy or stream) listen on their own ports *inside* the container. Publish
  each one with an additional `-p <port>:<port>` mapping, e.g. `-p 5432:5432`.
- **HTTPS / ACME (Let's Encrypt):** issue certificates under **Certificates**
  in the UI, then link one to a reverse-proxy server (TLS section of the
  server form). Prerequisites for issuance (HTTP-01): the domain's DNS A/AAAA
  record must point at this host's public IP, and ports **80** (challenge)
  and **443** (HTTPS serving) must be published as in the run command above.
  Certificates auto-renew ~30 days before expiry (checked at boot and every
  12h); issuance/renewal errors are shown on the certificate's detail page.
  The ACME account key, certificates, and private keys persist in the
  `/var/lib/nginx-web-ui` volume (also mirrored in Postgres as the source of
  truth). `ACME_DIRECTORY_URL` overrides the ACME directory (testing).
- **Editing a certificate:** a certificate's domains, challenge type, issuing
  target, and contact address can be changed in place (**설정 수정** on its
  detail page) — the certificate keeps its ID, so servers linked to it stay
  linked. Saving only changes the settings; the live certificate keeps serving
  until you run a renewal from the detail page. This is how you move a DNS-01
  certificate onto HTTP-01 to make it auto-renewing, without the old
  unlink → delete → reissue → relink dance.

### Automatic DNS-01 renewal (Cloudflare)

DNS-01 is normally the one issuance path that cannot renew itself, because each
renewal needs a fresh `_acme-challenge` TXT record — *while a human has to
publish it*. If the domain is on **Cloudflare nameservers**, the app can publish
and retract that record through the Cloudflare API, and DNS-01 becomes as
automatic as HTTP-01. That matters for domains that cannot expose port 80, which
is exactly why they were on DNS-01 in the first place.

On a certificate's **설정 수정** page (or when creating one), pick
**Cloudflare (자동)** under *TXT 자동 등록* and paste a Cloudflare API token with
**Zone:DNS:Edit** and **Zone:Zone:Read** on that certificate's zone.

The token belongs to the **certificate**, not to the installation: different
domains routinely live in different Cloudflare accounts, so one global token
would not cover them. Storing it per certificate also lets the save check
something a global setting cannot — that this token can actually see *this
certificate's* zones. A token that fails that check is rejected rather than
saved, so a broken setup surfaces immediately instead of at renewal time. The
token is encrypted at rest and never rendered back into the page.

From then on the certificate renews itself ~30 days before expiry: the app
creates the TXT records, waits for propagation, completes validation, and
deletes the records again — on success *and* on failure. The live certificate is
never touched until a renewal actually succeeds.

### Certificate renewal reminders (SMTP)

DNS-01 certificates **without** DNS automation still need a human, so they are
the one kind that can quietly expire. When such a certificate enters its 30-day
renewal window, the app emails the ACME account's contact address (at most once
a week per certificate, and it stops on its own once the certificate is
renewed). Certificates using Cloudflare automation are excluded — they renew
themselves.

Configure the mail server under the admin panel → **메일 (SMTP)**
(`/admin/smtp`, admin only). Settings that are switched on are connection-tested
before they are saved, so a configuration that cannot reach its server is
rejected rather than stored as a false sense of coverage. The stored password is encrypted at
rest with a key kept in the app-data volume (`DATA_KEY_FILE`, default
`/var/lib/nginx-web-ui/secrets/data_key`) — a database dump alone does not
reveal it, and the UI never renders it back.

### Blocking abusive clients (fail2ban)

`/var/log/nginx` and `/var/log/nginx-web-ui` are volumes so a fail2ban running
on the **host** can read them. (fail2ban is not run inside the container: it
needs `NET_ADMIN`, and bans applied in the container's network namespace would
not stop traffic arriving on published ports anyway.)

A failed sign-in is **invisible in nginx's access log**. The login action ends
in a redirect on both success and failure, so nginx records `POST /login 303`
either way, and the only thing that differs — the `Location` header — is not
part of the `combined` format. The app therefore writes its own line per failed
attempt to `/var/log/nginx-web-ui/auth.log` (override the path with `AUTH_LOG`):

```
2026-08-09T14:16:03.386Z nginx-web-ui auth-failure ip=203.0.113.5 reason=bad_credentials
```

`reason` is `bad_credentials`, `account_disabled`, or `ip_throttled`. The email
is deliberately left out — this file is mounted for fail2ban, and the IP is all
it needs. Which account was targeted is in the audit log instead.

Filter and jail files ship in the image; copy them to the host and reload:

```sh
docker cp nginx-web-ui:/usr/share/nginx-web-ui/fail2ban/filter.d/nginx-web-ui-auth.conf /etc/fail2ban/filter.d/
docker cp nginx-web-ui:/usr/share/nginx-web-ui/fail2ban/jail.d/nginx-web-ui.conf     /etc/fail2ban/jail.d/
# point the jail's logpath at wherever you mounted the log directories, then:
fail2ban-client reload && fail2ban-client status nginx-web-ui-auth
```

Four things to get right:

- **Mount the log directories to host paths** (e.g.
  `-v /srv/nginx-web-ui/log/app:/var/log/nginx-web-ui`) so fail2ban can read
  them, and set the jail's `logpath` to match.
- **Use `chain = DOCKER-USER`.** Traffic to published ports is DNATed and
  traverses FORWARD, so fail2ban's default `INPUT` rules never match it — bans
  show up in `fail2ban-client status` while nothing is actually blocked. The
  shipped jail sets this; verify with `iptables -L DOCKER-USER -n`.
- **Verify the filter** before trusting it:
  `fail2ban-regex /path/to/auth.log /etc/fail2ban/filter.d/nginx-web-ui-auth.conf`.
  A filter that matches nothing looks exactly like a quiet server.
- **Rotate the logs.** They persist across container recreation now, so they
  grow without bound; a sample logrotate config is in the doc below.

For the full walkthrough — mount examples, logrotate, and the edge/real-IP
caveat (with a CDN in front, enable each server's *real IP restore* setting or
you will ban the CDN) — see
[`docker/fail2ban/README.md`](./docker/fail2ban/README.md).

See [`.env.example`](./.env.example) for all runtime environment variables
(`DATABASE_URL`, `AUTH_SECRET`, `ADMIN_EMAIL`,
`ADMIN_PASSWORD`, `MIGRATIONS_DIR`, `AUTH_LOG`). Everything is optional — with no `-e`
flags the container generates a persistent `AUTH_SECRET`, uses the in-image
Postgres, and provisions the first admin via the web `/setup` screen
(`ADMIN_PASSWORD` is only used if you set it explicitly).

## Develop

```sh
yarn install          # from source/
yarn dev              # turbo run dev, all apps
yarn check-types      # tsc --noEmit across all packages
yarn test             # vitest, all packages
yarn build             # turbo run build (standalone Next.js build for apps/web)
```

Run a single app/package's script directly, e.g. `yarn workspace web run dev`
or `yarn workspace @repo/presentation run test`. The `@repo/db` workspace now stores
only committed SQL migrations; Rust runs and tests them.

For local development, run the Rust API against a disposable PostgreSQL database
using `DATABASE_URL` and an `AUTH_SECRET`. These are API-only inputs; do not put
them in the Next.js environment. Next uses `API_URL` (default
`http://127.0.0.1:3001`) for server-side API calls. The Docker Compose development
override provides the complete single-container environment.

Run the public Rust suite from `apps/api` with `cargo test --locked --all-targets`.
Cross-language compatibility tests are explicitly ignored: they require external
reference fixtures supplied through `NWUI_COMPAT_FIXTURES`,
`NWUI_NOTIFY_FIXTURES`, `NWUI_CONTROL_FIXTURES` (with `nginx/` and `acme/` children),
and `NWUI_NODE_OPENAPI_FIXTURE` (a JSON file). Select the relevant ignored tests
with `-- --ignored`; missing input fails the selected test. The native image
gate additionally requires `NWUI_NATIVE_IMAGE`, `NWUI_CI_REV`,
`NWUI_NATIVE_HOST_ROOT` and `NWUI_NATIVE_TEST_SCRIPT`. These private reference and
image gates are outside the default public test suite; they remain release checks.
