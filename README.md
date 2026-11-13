# nginx-web-ui

<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="source/packages/ui/src/brand/nginx-web-ui-logo-dark.png">
  <img alt="nginx-web-ui" src="source/packages/ui/src/brand/nginx-web-ui-logo-light.png" width="400">
</picture>

**English** · [한국어](docs/README.ko.md)

[Install](#1-requirements-and-image) · [Update history (Korean)](https://github.com/xelexis-dev/nginx-web-ui/blob/main/docs/UPDATE.md) · [Upgrade path (Korean)](https://github.com/xelexis-dev/nginx-web-ui/blob/main/docs/UPGRADE_PATH.md) · [Releases](https://github.com/xelexis-dev/nginx-web-ui/releases) · [Issues](https://github.com/xelexis-dev/nginx-web-ui/issues) · [License](#license-and-contributing)

</div>

**nginx-web-ui manages nginx from a web interface.** Instead of editing nginx configuration files, you set up reverse proxies (HTTP/HTTPS), TCP/UDP proxies, certificates, caching and gzip compression. It also provides user permissions, change history with restore, log viewing and management of multiple nodes.

The admin UI, nginx and the database ship in **one Docker image**. You do not install nginx or a database separately. You create the configuration files you need by copying them from this guide.

The admin UI is in Korean. This guide quotes on-screen labels in Korean, with an English gloss in parentheses the first time.

## Features

- **Proxies:** create and apply reverse proxies (HTTP/HTTPS) and TCP/UDP proxies without editing configuration files.
- **Certificates:** issue Let's Encrypt certificates with HTTP-01 or DNS-01 and attach them to proxies.
- **Caching and gzip compression:** configure them per server and turn them on or off per path.
- **Reaching targets:** 「실행 중에 이름 확인」 (resolve names at runtime) lets a configuration apply even while its target is down, and 「재시작 중 다른 인스턴스로 재시도」 (retry another instance during restarts) reduces interruptions while a target restarts.
- **Users and records:** user permissions, audit log, change history with restore, and log viewing.
- **Sign-in and notifications:** OAuth (OIDC) sign-in, passkey sign-in, and email and webhook notifications.
- **Multiple nodes:** manage several nodes.

## Contents

1. [Requirements and image](#1-requirements-and-image)
2. [Install with Docker Compose](#2-install-with-docker-compose)
3. [Create the first administrator](#3-create-the-first-administrator)
4. [Check the installation](#4-check-the-installation)
5. [Start using proxies](#5-start-using-proxies)
6. [Update and stop](#6-update-and-stop)
7. [Troubleshooting](#7-troubleshooting)
8. [Support and security reports](#8-support-and-security-reports)

## 1. Requirements and image

- You need Docker that can run Linux containers: Docker Engine with the Compose plugin on Linux, or Docker Desktop in Linux-container mode on Windows and macOS. [Install Docker and Compose](https://docs.docker.com/compose/install/)
- **Linux hosts need kernel 5.11 or later and a recent Docker Engine.** Check with `uname -r` on the Docker server. If the kernel is older, or an older Docker's system-call filter blocks a required operation, the container does not start and reports `isolation_unavailable`. A new installation never reaches the sign-in page, and an upgrade from 1.19.x restores PG15 and then waits, so keep such hosts on 1.19.11 and follow the 1.19.11 installation guide. Docker Desktop on macOS and Windows is not subject to the host kernel requirement.
- **Supported versions:** new fixes ship only for the 2.0 series. **1.19.11 is the last 1.x release.** There will be no further 1.x releases, security fixes included.
- **Environments 2.0.0 was verified on:** Docker Engine on an AMD64 Linux kernel, and OrbStack on macOS (ARM64). Docker Desktop on Windows and macOS was not verified with the final 2.0.0 build.
- The image address is `ghcr.io/xelexis-dev/nginx-web-ui`.
- The examples below use **2.0.0**.
- Changes per version are in the [update history (Korean)](https://github.com/xelexis-dev/nginx-web-ui/blob/main/docs/UPDATE.md), and new versions are announced on [Releases](https://github.com/xelexis-dev/nginx-web-ui/releases).

Use the following commands to check the Docker connection, the Compose installation, and **the architecture of the Docker server that will run the container**. With a remote Docker context, the machine you type on and the machine that runs the container can differ.

```sh
docker version
docker compose version
docker context show
docker info --format '{{.OSType}}/{{.Architecture}}'
```

| Docker server | Image to use |
| --- | --- |
| `linux/x86_64` or `linux/amd64` | `ghcr.io/xelexis-dev/nginx-web-ui:2.0.0` |
| `linux/aarch64` or `linux/arm64` | `ghcr.io/xelexis-dev/nginx-web-ui:2.0.0-arm64` |

**`latest` stays on 1.19.11 and does not move across major versions.** This keeps installations that use `latest` from moving to 2.0 by accident and starting a database migration. Install 2.0 with the version tags in the table above, and always pin a version tag for an installation in use. `latest` and version tags without a suffix are **AMD64 only**. For a new ARM64 installation, choose the `-arm64` tag. These are not multi-architecture tags that select the architecture for you. **When you update an existing installation, keep the architecture of the existing container's image, not the host's.** Section 6 shows how to check it.

Before you install, check existing containers with `docker ps -a`. The example below is for a new installation. If a `nginx-web-ui` container or `nwu-*` volumes already exist, first find out whether they belong to an existing installation. Do not delete a service in use, and do not reuse its data volumes for a new installation.

## 2. Install with Docker Compose

Create a folder for the installation and change into it. These commands work in both Bash and PowerShell.

```sh
mkdir nginx-web-ui
cd nginx-web-ui
```

In this folder, create a file named **`compose.yaml`** and save the following content as is. **On ARM64, change the `image` tag to `2.0.0-arm64` before you start it.**

```yaml
services:
  nginx-web-ui:
    image: ghcr.io/xelexis-dev/nginx-web-ui:2.0.0
    container_name: nginx-web-ui
    restart: unless-stopped
    stop_grace_period: 10m
    security_opt: ["no-new-privileges:true"]
    ports:
      - "8080:8080"
      - "80:80"
      - "443:443"
    volumes:
      - pg-data:/var/lib/postgresql/data
      - app-data:/var/lib/nginx-web-ui
      - nginx-log:/var/log/nginx
      - app-log:/var/log/nginx-web-ui

volumes:
  pg-data:
    name: nwu-pg
  app-data:
    name: nwu-app
  nginx-log:
    name: nwu-nginx-log
  app-log:
    name: nwu-app-log
```

`security_opt: ["no-new-privileges:true"]` stops processes from gaining higher privileges through the special permissions of system programs.

To create the administrator account without a browser, **add the `admin.env` file and the `env_file` setting from section 3 before you start the container**. To create the account in the browser, carry on as is.

In the same folder, check the configuration, pull the image and start it.

```sh
docker compose -f compose.yaml config --quiet
docker compose -f compose.yaml pull
docker compose -f compose.yaml up -d
```

Volumes are created automatically when they do not exist. Docker-managed volumes work on Linux, Windows and macOS. To use host folders on macOS, follow the notes below. The database permission behaviour of Windows host folders has not been verified, so use managed volumes there.

### Ports and addresses

Ports are written as `host port:container port`.

| Container port | Purpose | Default access |
| --- | --- | --- |
| `8080/tcp` | Admin UI | `http://localhost:8080` on the machine you installed on |
| `80/tcp` | HTTP proxy and Let's Encrypt HTTP-01 validation | HTTP requests for the domains you register |
| `443/tcp` | HTTPS proxy | HTTPS requests for domains with a certificate attached |

From another machine, open `http://<Docker server address>:8080`. Replace `<Docker server address>` with the address of the actual host. Firewalls and the network must also allow the ports you need.

If the admin port is already in use, change `"8080:8080"` to `"18080:8080"` and use `http://localhost:18080`. To allow access to the admin UI only from the machine you installed on, use `"127.0.0.1:8080:8080"`. Until the first administrator exists, make sure only the person installing can reach the admin port. Section 3 lists the address requirements for the first setup.

The internal web server `3000/tcp`, the API server `3001/tcp` and PostgreSQL `5432/tcp` are not published. Keep the container's default ports and internal process settings.

### Persistent data

| Docker volume | Container path | Contents |
| --- | --- | --- |
| `nwu-pg` | `/var/lib/postgresql/data` | The database: users, proxy settings, audit log, configuration history and more |
| `nwu-app` | `/var/lib/nginx-web-ui` | App data: authentication and encryption keys, the database secret, certificates and more |
| `nwu-nginx-log` | `/var/log/nginx` | nginx access and error logs |
| `nwu-app-log` | `/var/log/nginx-web-ui` | App logs, such as failed sign-ins |

**Never use the same database and app volumes from two containers at the same time.** To compare an old and a new version, use an independent restored copy.

**Keep and back up `nwu-pg` and `nwu-app` together.** If you delete only the app volume, the app may no longer connect to the existing database or read encrypted settings. Do not mount separate volumes over `/etc/nginx`, `conf.d` or `stream.d`. The app generates the nginx configuration from the stored logical settings and applies it again.

If you mount host folders on the log paths, the nginx logs are set to root:nginx 0750 and the app logs to root 0755. If an external program writes the logs, or an account other than root reads them, check the permissions. Log monitoring that reads as root keeps working.

**Logs are not cleaned up automatically.** `nwu-nginx-log` and `nwu-app-log` keep collecting every request that passes through the proxies and every app log, so watch the free disk space and rotate the logs on the host regularly. After you move or rename a log file, run `docker exec nginx-web-ui nginx -s reopen` so that nginx writes to the new file. Otherwise it keeps writing to the old file, and the traffic statistics may show 0. Emptying the file in place instead of moving it (logrotate's `copytruncate`) also works. The container's own output (`docker logs`) can be capped with the Compose `logging` settings, such as `max-size`.

### Host folders on macOS

<details>
<summary>Show details</summary>

Docker Desktop and OrbStack on macOS can use a Mac folder as storage. Docker Desktop must allow that folder under file sharing. Check that writes, file locking and data sync work correctly on that storage.

Mac shared folders in Docker Desktop and OrbStack do not enforce file permissions between users, so the API, the web UI server and the nginx worker processes can also read and change the database files, encryption keys, secrets, published configuration files and logs in that storage. If one of these processes is compromised, this can extend to root inside the container. The app shows a warning for storage it detects and continues installation and automatic upgrades. If you need permission separation between the processes inside the container, use Docker-managed volumes. On macOS, OrbStack with Docker-managed volumes is recommended where possible.

On shared folders, owner and group numbers can look different depending on who reads them, and a successful ownership change does not guarantee access restriction by number. Read-only storage, and mounts not confirmed as a supported sharing method, keep the existing checks. Mounting a path inside the OrbStack virtual machine directly is not confirmed as supported, so use Docker-managed volumes.

To use host folders, change the `volumes` of the Compose service above as follows. Replace `/absolute/path/nwui` with the real storage path, and with Docker Desktop make it accessible under file sharing. For a new installation, create the four folders first. An existing installation keeps the same database and app folders. PostgreSQL 15 (PG15) data is checked, backed up and migrated automatically the first time 2.0.0 starts. No other container may use the same folders at the same time.

```yaml
    security_opt: ["no-new-privileges:true"]
    volumes:
      - /absolute/path/nwui/postgres:/var/lib/postgresql/data
      - /absolute/path/nwui/app:/var/lib/nginx-web-ui
      - /absolute/path/nwui/nginx-log:/var/log/nginx
      - /absolute/path/nwui/app-log:/var/log/nginx-web-ui
```

Mount the database volume at `/var/lib/postgresql/data`. The actual 2.0.0 PG17 data is created in `17/data` inside it, so do not mount `17/data` itself, and do not override the path with the `PGDATA` or `PGROOT` environment variables. **A new installation needs both the database and app volumes to be empty.** An existing PG15 installation keeps its original mounts and follows the backup and automatic migration notes in section 6.

If an earlier image stored PG15 in `postgres/pgdata/PG_VERSION`, **keep mounting the parent `postgres` folder as before**. The app recognises both the root and the old `pgdata` subfolder layout, and when it is done it uses `17/data` under the same database root. Do not move folders or edit `PG_VERSION`. See the [macOS migration notes (Korean)](https://github.com/xelexis-dev/nginx-web-ui/blob/main/docs/UPGRADE_PATH.md#macos의-기존-하위-폴더).

If the log shows `pg-init: FATAL: PostgreSQL data ownership could not be set to postgres:postgres.`, startup stopped because the ownership could not be set. Check the write permission of the shared folder and the mount path. Do not delete existing data or switch to a new empty folder.

</details>

## 3. Create the first administrator

### In the browser

1. After the container has booted, open `http://localhost:8080/setup`. Adjust the address for a remote installation or a changed port. The first administrator is created at the admin address set in `AUTH_URL`, or in `NEXTAUTH_URL` when `AUTH_URL` is not set. If neither is set, you must connect through `localhost` or an IP address. To use a domain, set `AUTH_URL` to that address first.
2. Enter the email and password of the first administrator. The password must be **at least 8 characters long and contain both letters and digits**.
3. Sign in with the new account and check that the admin UI opens.

Create the first administrator right after installation, or create it in advance with the environment variables below. Until the first setup is done, only the person installing should be able to reach the admin port.

There is no default password. The administrator password is never generated automatically or printed to the logs. Once a user exists, the first setup page can no longer be used.

### Signing in with a passkey

After you register a passkey under 「패스키」 (passkeys) on the account page, you can sign in with 「패스키로 로그인」 (sign in with a passkey) on the sign-in page, without an email or password. Registering and removing a passkey need your current password, and each account can have up to 10. Passkeys work only when you connect through one of these addresses:

- an HTTPS domain that this product's reverse proxy connects to the admin UI (port `8080`)
- the HTTPS address set in `AUTH_URL`, behind an external HTTPS proxy
- `http://localhost` on the machine where nginx-web-ui is installed

When you connect by IP address or over plain HTTP, passkeys cannot be used and the sign-in page shows no passkey button. If `AUTH_URL` is set, passkeys work only at that address. A passkey works only at the address it was registered at, including the port. If you change the address or port, register a new passkey at the new address. Password sign-in stays available, subject to the configured admin address: if `AUTH_URL` is set, sign-in is accepted only at that address.

### Without a browser (agents and automation)

To skip the first setup in the browser, pass an email and password chosen by the operator **before the first start**. Create `admin.env` in the same folder as `compose.yaml` and replace the two values with real ones.

```dotenv
ADMIN_EMAIL=admin@example.com
ADMIN_PASSWORD='REPLACE_WITH_YOUR_UNIQUE_PASSWORD_123'
```

Do not use the example password. In a Compose environment file, a password in single quotes passes characters such as `$` through unchanged. Only the person installing should be able to read the file, and it must not be committed to a repository. [Compose environment file syntax](https://docs.docker.com/compose/how-tos/environment-variables/variable-interpolation/#env-file-syntax)

Merge the following settings into the `nginx-web-ui` service in `compose.yaml`. `env_file` is indented at the same level as `image`, `ports` and `volumes`. Write the existing `security_opt` only once.

```yaml
    security_opt: ["no-new-privileges:true"]
    env_file:
      - ./admin.env
```

Then run the `pull` and `up -d` commands above. With an empty database, the account is created and you use the sign-in page instead of `/setup`. If users already exist, these environment variables do not change any password.

A basic installation needs no other environment variables. If you do not set `AUTH_SECRET`, it is generated on the first boot and kept in the app volume; if you set it, your value is used. On later starts too, an `AUTH_SECRET` or `ADMIN_PASSWORD` that you set takes precedence over the stored value. Changing `ADMIN_PASSWORD` does not change the password of a user that already exists. The built-in database secret is also kept in the app volume, so do not set `DATABASE_URL` or prepare an external database.

If you use the admin UI only through an HTTPS domain behind an external reverse proxy, or use OAuth (OIDC) sign-in, you can optionally set `AUTH_URL` to that address (for example `https://admin.example.com`). When it is set, sign-in and configuration change requests are accepted only from that address. The address used to create the first administrator follows the conditions in *In the browser* above.

An empty `AUTH_URL`, or an empty `NEXTAUTH_URL` (the compatibility setting), counts as not set. Use an HTTP(S) address that includes a host. A valid `AUTH_URL` takes precedence, and `NEXTAUTH_URL` is then not used. An invalid address is reported with a warning and treated as not set, so check that external sign-in and the admin address work as intended.

Admin request handling, the UI server and the target name resolution service run with the regular user permissions they need. Existing installations do not need to change the run user, and the container starts with its default administrator privileges. Do not expose the secret files in the app data, do not widen shared folder permissions across the board, and do not add an arbitrary `user:` to Compose. If the ownership conditions of the storage are not met, the app keeps the data and refuses to initialise or migrate it.

An existing `DATA_KEY_FILE` does not require a particular file mode. The admin request user must not own the file or be able to write to it, and any world-writable parent folder must be protected with the sticky bit. **In an installation without a key, a custom path that is read-only is refused before anything in the original is changed.** Do not delete the existing key file or replace it with an arbitrary new key. With macOS host folders in Docker Desktop or OrbStack, a custom `DATA_KEY_FILE` must be inside the app storage (the default path, or under `/var/lib/nginx-web-ui`) or on a Docker-managed volume. A key file in a host folder mounted anywhere else is refused, because its access restriction cannot be confirmed. A 2.0 start reports this as `pair_mismatch`, and an upgrade from 1.19 as `key_path_invalid`. The data is not changed.

### Showing the admin UI inside another site

<details>
<summary>Show details</summary>

By default, only frames from the same origin are allowed. To show the admin UI inside another site, list the allowed HTTP(S) origins, separated by spaces, in the `environment` of the Compose service. Do not include paths or wildcards.

```yaml
    environment:
      NWUI_FRAME_ANCESTORS: "https://portal.example.com https://ops.example.com"
```

A missing or invalid value keeps the default, which allows the same origin only. IPv6 address notation is not supported, so use a domain or IPv4. This setting only allows framing; it does not skip sign-in or permission checks.

</details>

## 4. Check the installation

```sh
docker compose -f compose.yaml ps
docker compose -f compose.yaml logs --tail=100 nginx-web-ui
docker exec nginx-web-ui nginx -t
docker exec nginx-web-ui pg_isready -h 127.0.0.1 -p 5432
```

`nginx -t` must report a successful configuration test, and `pg_isready` must print `accepting connections`. A container shown as `running` does not by itself mean that the web service is ready. If it is still initialising, wait a moment, then check over HTTP as well.

In Bash, use this command:

```sh
curl --fail --silent --show-error --location --output /dev/null --write-out '%{http_code}\n' http://localhost:8080/setup
```

In PowerShell, use this command:

```powershell
(Invoke-WebRequest -Uri 'http://localhost:8080/setup').StatusCode
```

A new installation that booted correctly returns **HTTP 200** from the first setup page. If an administrator already exists, you are taken to the sign-in page. Finally, check that you can sign in with a real account. If you used a remote Docker server or a different host port, change the URL you check as well.

## 5. Start using proxies

### Ports and targets

- The nginx worker processes run as the `nginx` user. If a target is a Unix socket mounted from the host, this user needs permission to connect to the socket and to pass through every parent folder; otherwise requests get 502.
- In the admin UI, register upstreams (target services), set the domain, port and target of a reverse proxy, then apply. Caching and gzip compression are configured per server and can be turned on or off per path.
- `localhost` and `127.0.0.1` entered as a proxy target mean **the nginx-web-ui container itself**. For other containers, connect them to the same Docker network and use the service name and internal port. For external services, use an address and port the container can reach. [Docker networking](https://docs.docker.com/engine/network/)
- Adding a port in the admin UI does not publish it in Docker. For example, a stream that uses TCP `15432` in the container needs `"15432:15432/tcp"` under `ports`, and UDP `15353` needs `"15353:15353/udp"`. Then run `docker compose -f compose.yaml up -d` again. TCP and UDP must match as well.
- To issue a Let's Encrypt HTTP-01 certificate, the domain's DNS A/AAAA records must point to this host, and **port 80** must be reachable from the internet. Attach the issued certificate to a proxy to serve HTTPS on **port 443**. Where HTTP-01 cannot be used, choose DNS-01 on the 「인증서」 (Certificates) page.
- Before you register a proxy domain, a **404 from `http://localhost:80` is normal**. The admin UI is on `:8080` by default. HTTPS requests on 443 for a domain without a certificate may show a default certificate warning.

### WebSocket headers

When you create or edit a reverse proxy path, 「**WebSocket 헤더 채우기**」 (fill WebSocket headers) fills in HTTP/1.1 and the Upgrade and Connection headers. For requests that are not upgrade requests, Connection is set to close. If they conflict with existing values, review them and choose to replace them. Host, other headers, timeouts and buffering settings are kept.

### Resolving target names at runtime

<details>
<summary>Show details</summary>

You cannot write `resolver` or `resolver_timeout` directly in the extra directives. Use 「실행 중에 이름 확인」 instead.

If you choose 「**실행 중에 이름 확인**」 for a reverse proxy's direct target or path, an upstream, or a TCP/UDP stream, the configuration can be applied even while the target is down and its name cannot be found. Once the target is ready, connections are tried again, and you do not need to apply again when the address behind the name changes. Existing items are not switched to this mode automatically.

While a name cannot be resolved or reached, HTTP requests may get 502 or 504, TCP connections may be closed, and UDP may get no response. Resolution results are kept for a short time, so a target starting or changing its address does not reach every request immediately. Do not turn this option on for a direct target that is an IP address. Paths that use an upstream registered by name follow that upstream's setting.

To reach another container by name, put both containers on the same Docker network. For example, another container's `px-web:3001` works, but loopback, unspecified, multicast and link-local addresses returned by dynamic resolution are not allowed. Aliases added only to the container's `/etc/hosts` are not used for runtime DNS resolution. The existing static target `http://127.0.0.1:8080` for the admin UI keeps working.

By default the container's DNS servers are used, with Docker's DNS first when it is present. If you need the settings below, add them to the `environment` of the Compose service.

| Environment variable | Default / behaviour | What to check when you use it |
| --- | --- | --- |
| `NWUI_TARGET_DNS_PORT` | TCP/UDP `5354` inside the container | It must not clash with existing ports, including proxies and streams that are turned off. On a clash, choose a free port and restart the container. This port is not meant to be published. |
| `NWUI_TARGET_DNS_SERVERS` | The container's DNS servers | 1–3 IP addresses or `IP:port` values, separated by commas or spaces. Without a port, `53` is used; for IPv6 with a port, write `[IPv6]:port`. It never switches to a public DNS service on its own. |
| `NWUI_TARGET_DNS_IPV6` | Resolves both IPv4 and IPv6 targets | Set it to `off` where IPv6 connections to targets are not available. |

For `NWUI_TARGET_DNS_SERVERS`, enter the real IP of a DNS server that is reachable from your environment. For example, add one of the following lines under `environment` in Compose.

```yaml
# IPv4 example: default port and an explicit port, comma-separated
NWUI_TARGET_DNS_SERVERS: "10.0.0.53,10.0.0.54:5353"
# IPv6 example: brackets for an address with an explicit port, space-separated
NWUI_TARGET_DNS_SERVERS: "[fd00::53]:5353 fd00::54"
```

If, during an update, the default internal DNS port clashes with an existing item, the existing static configuration is kept and the reason for the clash is shown. Runtime name resolution cannot be newly turned on until the operator resolves the port clash. If the internal DNS service stops briefly, new resolutions may fail, and connections may keep using previously resolved addresses for a while.

</details>

### <a id="대상-서비스를-하나씩-재시작할-때-끊김-줄이기"></a>Fewer interruptions when restarting targets one at a time

<details>
<summary>Show details</summary>

With a single target container, nothing can take requests while it restarts. With the following preparation, another instance can handle requests while one instance restarts.

1. **Run two or more instances of the same service.** For example, if the target service is `web`, you can scale it with `docker compose up -d --scale web=2 web`. Do not set a fixed `container_name:` on the target service. When one stops, the remaining instances must be able to handle all requests, and sign-in state and shared data must be kept outside the instances, for example in a database. This does not run several copies of nginx-web-ui itself.
2. **Address the target by service name and turn on 「실행 중에 이름 확인」.** Connect nginx-web-ui and the target service to the same Docker network, and enter the service name and internal port instead of a container IP (for example `http://web:3000`, or `web:3000` in an upstream). Check that the service name returns the addresses of all running instances. With this option, address changes are followed without applying again, but address updates may not finish immediately. With the option off, the first resolved address keeps being used, so after a container is recreated, requests may go to another service that took over that address. For paths that use an upstream, turn the option on in that upstream.
3. **Turn on the path's 「재시작 중 다른 인스턴스로 재시도」.** On a connection failure, a retryable timeout, or a 502, 503 or 504 response, the request is sent once more to another instance. That is **at most 2 attempts**, including the first, and **the next attempt can start within 3 seconds of the first one**. The 3 seconds are not a limit for completing the whole request. Connecting waits up to **2 seconds**, or the connect wait you set for the path. Waiting for the response after connecting follows the read limit. If the path's connect wait is 3 seconds or more, a connect timeout may not move on to another instance. With a single instance, the request cannot be sent elsewhere, but the connect wait limit still applies. This switch is off by default, and with it off, nginx's default retry behaviour remains.
4. **Prepare the app to handle the stop signal.** On `SIGTERM`, it must stop accepting new connections, finish the requests in progress, and then exit. In a shell script, start the app with `exec` at the end, or use a launcher that forwards the signal to the app and waits until the app exits after finishing its requests. Compose's `init: true` helps clean up leftover child processes, but it does not forward signals or finish requests for you. Also set `stop_grace_period` comfortably above the request handling time.
5. **Replace one at a time, and check that requests through nginx reach the new instance.** After replacing one, first wait **at least 20 seconds**. This is only an initial wait for address updates and does not guarantee readiness. Before stopping the next instance, check that the new instance is ready and that **requests through nginx-web-ui actually reach it**. You can check this in the new instance's access log, or with an instance identifier in its responses. If you cannot confirm it, do not stop the next instance. `docker compose restart web`, or `docker compose up -d` after an image update, may restart or recreate all instances together, so first check that your method replaces them one at a time. Adding a `healthcheck` does not make nginx read that status.

**Limits to keep in mind**

- POST and PATCH requests that were already sent are not sent to another instance. A request may fail if the instance is forcibly stopped while handling it, so graceful shutdown must be supported. For work that must not run twice, such as payments or sign-ups, the app must also prevent duplicates. GET, PUT and DELETE may be forwarded again, so set them up so that repeating the same request does not change the result.
- If all instances stop or do not respond, requests may fail with **502 or 504**. Depending on the connect wait and read limit, this can take a while, so it does not always fail quickly. If clients retry too, limit the number of retries so that requests do not pile onto the remaining instances.
- An instance that accepted the connection but does not respond is waited on until the read limit. **If waiting to read has gone past the 3-second retry start limit**, the request is not sent to another instance. If a short read limit fails within 3 seconds, a retry is possible.
- WebSocket, streaming and TCP connections that are already established are not moved to another instance. Configure clients to reconnect.
- TCP/UDP streams do not have this switch. A TCP stream connection started while the target restarts may wait until the connect timeout. With UDP, a stopped instance cannot be detected as a connection failure.
- In environments such as Docker Swarm, where a service name points to a single virtual IP, nginx cannot tell the individual instances apart. Check that the service name really returns several addresses.

</details>

## <a id="6-업데이트와-중지"></a>6. Update and stop

**1.19.x migrates the database automatically when you start it with the new image tag and keep the existing volumes and settings.** 1.19.0–1.19.11 need no intermediate version and no separate manual migration. 1.18.x is outside the directly verified migration range, so going through 1.19.11 is recommended, but it is not required. **Below 1.18.0, first update to 1.19.11 and then continue.** During the first switch from PG15 to PG17, the proxy sites are stopped and the admin port shows only the progress status.

During the automatic database migration, turn off tools such as autoheal that restart the container when a health check fails. If you need to stop it, wait up to 600 seconds, for example with `docker stop -t 600 nginx-web-ui`, or set `stop_grace_period: 10m` in Compose. Do not force it to stop right after a stop request; check the shutdown result in the logs.

If the container restarts during the migration, the app checks the preserved records and recovers or continues the start. If the original must be restored before the switch to PG17, PG15 is verified and restored, and that start then waits, so follow the next step shown. After the switch to PG17, PG17 is kept and the start is prepared again. A restart right after success does not select the kept PG15 automatically. Keep the same database and app volumes, and do not delete the progress records.

1. Read the [update history (Korean)](https://github.com/xelexis-dev/nginx-web-ui/blob/main/docs/UPDATE.md) and the [upgrade path (Korean)](https://github.com/xelexis-dev/nginx-web-ui/blob/main/docs/UPGRADE_PATH.md). Keep the old image, Compose file and environment settings together with a database and app backup pair taken at the same point in time, and check that the backup can be restored. For the actual commands, follow 「중지 후 DB·앱을 함께 백업하고 복원하기」 (stop, then back up and restore the database and app together) in the [upgrade path (Korean)](https://github.com/xelexis-dev/nginx-web-ui/blob/main/docs/UPGRADE_PATH.md). The automatic PG15 backup does not replace an independent backup.
2. Check the architecture of the existing image, stop the container cleanly, then back up. Make sure no other container uses the same database and app volumes at the same time.
3. Change the image tag in the existing Compose file to `2.0.0` (AMD64) or `2.0.0-arm64` (ARM64). Keep the volume names, mounts, ports and authentication settings. If you pinned it with `@sha256:…`, replace the whole image address in `image:` instead of changing only the tag. If the old digest remains, the old image runs. Do not create new empty database or app volumes.
4. Pull the image, start it, and check the progress status on the admin port and the logs. Prepare free space of roughly 4 × the database size + 2 × the largest index + max(1 GiB, 1/5 of the database size), plus a fixed reserve and room for a copy of the app data. Even a small database needs about 1.3 GB or more. If the database and app are on the same file system, add the amounts together. Prepare according to the required space (`requiredBytes`) and inodes shown in the status. On separate file systems, each must meet its own space and inode requirement. Even if the shown total leaves room, a shortfall on one side cannot be covered by space on the other, so follow the low-space notice and free space on that storage. On macOS, also check the Docker Desktop disk limit.
5. When it is done, click the account button at the top right of the screen and check that the menu that opens shows version **2.0.0**. Then check sign-in with an existing account, real HTTP/HTTPS and TCP/UDP responses, certificates, notifications and node connections. If an existing installation shows the first administrator setup page, stop and check the data mounts first.


If the existing container has a different name, change the name below. **Keep the architecture of the existing image, not the host's.** Switching to another architecture while keeping the database files is not supported.

```sh
docker image inspect "$(docker inspect nginx-web-ui --format '{{.Image}}')" --format '{{.Architecture}}'
docker compose -f compose.yaml stop -t 600
docker logs --tail=100 nginx-web-ui
# After a clean stop, back up the DB and app data at the same point in time as UPGRADE_PATH.md describes, then change the image tag
docker compose -f compose.yaml config --quiet
docker compose -f compose.yaml pull
docker compose -f compose.yaml up -d
docker logs --tail=100 nginx-web-ui
docker exec nginx-web-ui nwui-db-upgrade auto-status
```

Check the progress directly on the existing host admin port (by default `http://<Docker server address>:8080`). An admin domain that goes through ports 80/443 of the same container cannot be used during the migration. Do not take a container shown as running to mean that the migration and service start are finished.

Even when the container is stopped and the admin UI cannot open, you can check the status with the same database and app volumes. The following is a read-only check for a default installation that keeps the stopped existing container. On ARM64, change the image tag to `2.0.0-arm64`, and also match any storage path environment variables you set separately to the existing installation.

```sh
docker run --rm --pull=never --security-opt no-new-privileges:true --network none \
  --volumes-from nginx-web-ui \
  --entrypoint /usr/local/bin/nwui-db-upgrade \
  ghcr.io/xelexis-dev/nginx-web-ui:2.0.0 auto-status
```

If cleanup remains after the migration, the status shows 「정리 대기」 (cleanup pending). A healthy service that is only waiting for cleanup is not an upgrade failure, and this state alone does not send the 「업그레이드 후 서비스 시작에 실패했습니다」 (service failed to start after the upgrade) notification. The size of the kept PG15 backup and the command to remove it are shown separately, and failed or waiting states show the next step.

The 「PG15 백업을 보관하고 있습니다」 (a PG15 backup is being kept) notice created during the upgrade is in the app's notification list. Email and webhooks are delivered only to targets already subscribed when the notification is created, so the first backup notice created while migrating from 1.19 is not delivered externally. To receive later notifications, open 「관리자」 (Admin), choose the 「메일」 (Mail) or 「Webhook」 tab, and select the events under 「시스템」 (System) when creating or editing a recipient or webhook. The 「업그레이드 후 서비스 시작에 실패했습니다」 notification is delivered to subscribed email and webhook targets.

### Holding, retrying and failures

<details>
<summary>Show details</summary>

The two environment variables take effect **only while an upgrade from PG15 is waiting**, and are read once when the container starts.

- `NWUI_DB_UPGRADE=off`: holds the migration and leaves the service waiting. The admin port shows a status page. Remove the variable and restart the container to continue the migration.
- `NWUI_DB_UPGRADE_RETRY=<identifier>`: requests exactly one retry for each distinct value. Setting the same value again does not request another retry with it. The value is a non-empty string of up to 128 characters and cannot contain NUL. Find the cause before you change the value.

If an automatic migration fails and the original PG15 is verified and restored, the result is shown, and that start waits instead of migrating again. Whether the next start can retry, when it can, and whether a new retry value is needed: follow the next step shown.

- **Original unchanged:** fix the cause shown and start again, or go back to the 1.19.11 image with the same volumes.
- **Original PG15 restored:** check the restore result and follow the retry conditions shown, or go back to the 1.19.11 image with the same volumes.
- **Restore not finished:** do not downgrade the image; check the preserved location and the error. Fix the cause, such as space or permissions, and restart the same 2.0.0 image to finish the recovery.
- **Service failed to start after the switch to PG17:** PG17 is kept. Restart to continue the start, or use the next explicit PG15 restore.
- **Database migration finished, waiting to confirm the configuration was applied:** the service has started, but the configuration apply status is not confirmed yet, so cleanup of the old data is held. This is different from a failed service start; the status is checked again and the cleanup continues.
- **Only cleanup of old data held, service healthy:** normal operation continues, and the remaining cleanup resumes at the next container start. This is not the step that deletes the separately kept PG15 backup automatically.

Migration diagnostic logs are kept in `<pgdata volume>/.nwui-auto-work/logs/<attempt>`, for the latest attempt only, and remain after a restore. Database connection failure logs show a short reason that tells cases apart, such as a rejected password or an unreachable server, and never contain the connection secret. `<pgdata volume>` is the mounted database volume and `<attempt>` is the attempt identifier.

If the status page does not open, check `docker logs --tail=100 nginx-web-ui`. Do not delete the records, and do not edit PG17 files to look like PG15.

</details>

### Returning to 1.19.11 after success, and removing the backup

<details>
<summary>Show details</summary>

**After a successful upgrade, do not just lower the image tag.** The kept PG15 must be restored first, and all writes since the upgrade are lost. Check the restore status and the real backup ID, stop the container, then run the command from the 2.0.0 image. Below is an example for a default installation. The database and app mounts and the storage path environment variables must match the existing installation. `restore-pg15` reads the data encryption key. The default key is in the app volume and is connected by the mounts below. If you use a custom `DATA_KEY_FILE`, add `-e DATA_KEY_FILE=<the same absolute path as before>` to the function's `docker run`, and keep the mount of that key file. `--volumes-from` does not copy environment variables. `auto-status`, `remove-pg15-backup` and `remove-discarded` do not read the key.

```sh
docker exec nginx-web-ui nwui-db-upgrade auto-status
docker compose -f compose.yaml stop -t 600
NWUI_UPGRADE_IMAGE=ghcr.io/xelexis-dev/nginx-web-ui:2.0.0
NWUI_BACKUP_ID=REPLACE_WITH_STATUS_BACKUP_ID

nwui_upgrade() {
  docker run --rm --pull=never --security-opt no-new-privileges:true --network none \
    --volumes-from nginx-web-ui \
    --entrypoint /usr/local/bin/nwui-db-upgrade \
    "$NWUI_UPGRADE_IMAGE" "$@"
}

nwui_upgrade restore-pg15 --confirm-loss "$NWUI_BACKUP_ID"
```

`restore-pg15` prints the next step that matches a finished restore and its retry conditions. Start 1.19.11, or, when you start the 2.0 image again, follow the wait time and new `NWUI_DB_UPGRADE_RETRY` value conditions shown. For the exact messages, see the [upgrade path (Korean)](https://github.com/xelexis-dev/nginx-web-ui/blob/main/docs/UPGRADE_PATH.md).

If the existing image is ARM64, change the tag in the variable to `2.0.0-arm64`. `--volumes-from` connects the mounts of the stopped container but does not copy its environment variables. If your installation sets storage paths separately, pass those environment variables explicitly as well. Only after the restore command succeeds, change the Compose image address to `1.19.11`, or `1.19.11-arm64` for the same architecture, and run `pull` and `up -d`. If you pinned a digest, replace the whole image address with the 1.19.11 address you kept instead of changing only the tag. If the 2.0.0 digest remains, 2.0.0 runs again. PG17 data that was set aside is not deleted automatically.


The PG15 backup remains until the operator removes it. Remove it with the real ID shown by `auto-status` only after you have confirmed that 2.0.0 starts normally and that you have an independent backup, and have decided that you do not need to return to PG15. **After removal, you cannot restore PG15 from that backup.** This is a separate cleanup example that uses the function and image above; it is not a procedure to run straight after the restore command.

```sh
docker exec nginx-web-ui nwui-db-upgrade auto-status
docker compose -f compose.yaml stop -t 600
nwui_upgrade remove-pg15-backup --confirm "$NWUI_BACKUP_ID"
docker compose -f compose.yaml start
```

Restore and cleanup commands **succeed with exit code 0; anything else is a failure**. Check the message together with `auto-status`. A finished PG15 restore consumes the backup by moving it into the current data, so `remove-pg15-backup` can no longer be used. Only the PG17 data set aside by the restore can be removed separately, with `remove-discarded --confirm <id>` using `discarded[].id` from the status. Both commands need the service to be stopped and the same database and app mounts. They do not delete your independent backup. For the full failure, retry and set-aside data cleanup notes, follow the [upgrade path (Korean)](https://github.com/xelexis-dev/nginx-web-ui/blob/main/docs/UPGRADE_PATH.md).

</details>

### Later updates and stopping

Later updates on PG17 work the same way: read the notes for that version, back up the database and app data together, and change the image tag with the same architecture and volumes. Gateways and nodes can be updated step by step with 2.0 and 1.19 mixed, but older nodes cannot use new features. Within 2.x, update the gateway first, then the nodes. A node accepts a gateway of a newer minor version than itself, but a gateway does not manage nodes of a newer minor version than itself. If you start an older image on a database that a newer version has used, it does not start and reports `database_newer`, so to go back, restore the backup pair taken before the update. If you save an item that uses 2.0-only settings in the 1.19 admin UI, the save is refused instead of the settings being deleted.

If you installed the 2.0.0 or 2.0.1 image published before 2.0.0 was reissued, set the image tag to `2.0.0`, also when you were on 2.0.1, and pull it again. The tag name is the same as the earlier 2.0.0, so without a new pull the earlier image keeps running. Back up first, as above. The first start adds the database structure for passkeys, and after that the earlier images do not start on that database (`database_newer`). After the update, the account page shows the 「패스키」 (passkeys) section.

When the 2.0.0 image published before the reissue is stopped, a single `Error: interrupted: 시작 준비를 중단했습니다…` (start preparation was interrupted) line may be left in the log even after a clean shutdown. If the exit code is 0 and the database shutdown log (`database system is shut down`) is present, the shutdown was clean. The current 2.0.0 leaves only the single line `[upgrade] 중지 요청을 받아 종료합니다.` (stop request received, exiting) instead of that error line.

To pause and start again:

```sh
docker compose -f compose.yaml stop -t 600
docker compose -f compose.yaml start
```

To remove the container but keep the data, use `docker compose -f compose.yaml down`. **`down -v` also deletes the data volumes, so do not use it for a normal stop or update.**


## 7. Troubleshooting

| Symptom | What to check |
| --- | --- |
| Cannot connect to Docker | Whether the Docker service is running, the current context, and permission to run Docker commands |
| `port is already allocated` | Which service uses that port on the host. For the admin port, change the host-side number. Do not stop existing services blindly |
| Architecture mismatch, `exec format error` | For a new installation, compare the Docker server and image architectures. For an existing installation, keep the same architecture using the existing container image check in section 6 |
| Image download fails | The image address and version tag, and network, DNS and HTTPS access to the registry |
| Container is running but the admin UI is unreachable | `docker logs --tail=100 nginx-web-ui`, the `nginx -t` and `pg_isready` results, host ports and the firewall |
| New installation, but `/setup` does not appear | Whether an existing database volume or `ADMIN_PASSWORD` has already created an account |
| Proxy returns 502 or 504 | The target address, port and Docker network. With runtime name resolution on, check that the target has started, the DNS response, and the internal DNS service. `localhost` means this container itself |
| Errors about the database password or encryption key | Whether `nwu-pg` and `nwu-app` are volumes of the same installation. Do not delete or replace only one of them; check the backup in which both were kept together |

### When startup is refused because of data paths

<details>
<summary>Show details</summary>

The new image may stop instead of creating a new key or an empty database on wrongly connected data. Even when it is refused at the startup check before any recovery record exists, it shows the cause and waits, so fix the cause and restart the container. The status page may remain, or the admin UI itself may not open, so read the message with the following command.

```sh
docker logs --tail=100 nginx-web-ui
```

| Code | What to check |
| --- | --- |
| `insufficient_space` | Compare the required space and inodes shown in the status with the available space. Fix the cause and restart the same image |
| Restore not finished | Check the preserved location and the error, and continue the recovery in the same 2.0.0 image. Do not downgrade the image before the restore is finished |
| `storage_busy` | Whether a container or admin command is using the same database and app volumes. Stop the concurrent use |
| `nginx_test_failed` | Check the file named in the nginx error lines of the log. Fix the errors in the base configuration or additional included files, then restart. Startup can also stop when isolation of user configuration could not finish or the final check failed |
| `trusted_input_unsafe` | Whether the user that handles admin requests owns, or can write, the nginx executable, configuration or included files named in the error |
| `pair_mismatch` | The database and app data do not belong together, or a required key is missing. With macOS host folders, a custom `DATA_KEY_FILE` placed in a separate host folder outside the app storage also fails the key access restriction check and shows this code, so move the key into the app storage or onto a Docker-managed volume. Do not replace only one side or create a new key. |
| `unsupported_layout` | An unsupported version or path, or other files, are present. Do not delete the original; check the real database root and the mounts. It can also occur when a path inside the OrbStack virtual machine is mounted directly. Keep the original database and app data and check the mounts. Docker-managed volumes are recommended for new installations. |
| `storage_override` | The existing `PGDATA` or `PGROOT` settings differ from the new storage path. Recreate the container from the Compose file instead of with a tool that copies the old container's settings, and remove the environment variables copied from the old image. The data is not moved. |
| `verification_failed` | A verification failure, for example of the image build information. If it is the build information, remove environment variables such as `APP_GIT_SHA` copied from the old image and recreate the container from the Compose file. Otherwise, check the cause shown in the log. |
| `isolation_unavailable` | Check that the Linux host has kernel 5.11 or later and a recent Docker Engine. It also occurs when a system-call filter blocks a required operation. An access restriction failure on a host folder not confirmed as a supported sharing method is another cause, so use Docker-managed volumes. If you cannot provide such an environment, stay on 1.19.11. |
| `database_newer` | A newer image has used this database. The data was not changed. Start with a newer image, or, to go back to an older version, restore the database and app backup pair made before the update, then start. |
| `log_dir_unsafe` | At every start, the log folders are set to root:nginx 0750 for nginx logs and root 0755 for app logs, and access restriction is checked. Detected Docker Desktop and OrbStack shared log folders may show a different owner and group, and an access restriction failure is shown as a warning, but the requested modes (0750/0755) are kept. Root-squash NFS, CIFS and read-only mounts, which cannot guarantee this, are refused, so use storage that supports permissions. |

Startup is refused if the user that handles admin requests (UID/GID 10001) owns or can write the executables, base configuration, modules or included files, including the nginx configuration directory. Write access through the nginx and adm groups and through ACLs is checked as well. Root ownership is not required; fix the owner, write permission and ACL conditions of the path named in the error. Adjust the configuration directory so that the user that handles admin requests cannot write to it, then start again. Do not solve it by widening permissions on all the data.

</details>

The address of a 2.0 node can be changed after it is confirmed to be the same node. If an existing connection has no confirmation information, refresh its status at the original address. For a 1.19 node, or an address confirmed to be a different node, disconnect it and connect again with a new token.

## 8. Support and security reports

- Report installation and update problems and bugs in [Issues](https://github.com/xelexis-dev/nginx-web-ui/issues). Including the version, image architecture, Docker engine (Docker Engine, Docker Desktop, OrbStack and so on), storage type (Docker-managed volume or host folder), and any code shown on screen or in the logs makes it quicker to look into.
- Do not post security vulnerabilities in public Issues; report them through [private security reporting](https://github.com/xelexis-dev/nginx-web-ui/security/advisories/new).
- When you attach logs, leave out passwords, tokens, encryption keys, certificate private keys and backup files, and mask values you do not want to make public, such as domains and addresses.

## License and contributing

- nginx-web-ui is distributed under the [MIT License](https://github.com/xelexis-dev/nginx-web-ui/blob/main/LICENSE). See [NOTICE](https://github.com/xelexis-dev/nginx-web-ui/blob/main/docs/NOTICE) for its scope and [THIRD_PARTY_NOTICES.md](https://github.com/xelexis-dev/nginx-web-ui/blob/main/docs/THIRD_PARTY_NOTICES.md) for the notices of included third-party components.
- See [CONTRIBUTING.md](https://github.com/xelexis-dev/nginx-web-ui/blob/main/docs/CONTRIBUTING.md) for how to contribute and [SECURITY.md](https://github.com/xelexis-dev/nginx-web-ui/blob/main/docs/SECURITY.md) for how to report security issues.
