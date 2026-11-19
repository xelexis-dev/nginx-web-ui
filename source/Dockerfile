# syntax=docker/dockerfile:1.28.0@sha256:bb22d9815c728170f72750f4e5b0d672e06176142e1d602c7e66c050100b7e5b

# Version-managed build/runtime inputs are fixed, including transitive Debian
# packages. snapshot.debian.org retains historical states; availability is not
# an SLA. Builds fail on a missing/different input, never fall back to live apt.
# Input refreshes belong to the next approved release with version + UPDATE.md.
# Trixie/PG17 runtime; 1.x data must first be migrated by the latest 2.0.x image.
ARG NODE_IMAGE=node:24.22.3-trixie-slim@sha256:af02d57b57719b9657a45199a79264a9437940cd55b3c7b3fc99615654f124a0
ARG RUST_IMAGE=rust:1.98.1-slim-trixie@sha256:4cd829461bd5c4d511c32e269da9cb8929223b666519d8004e35fc8d1d771ab7
ARG DEBIAN_SNAPSHOT=20261107T082527Z
ARG DEBIAN_SECURITY_SNAPSHOT=20261113T212035Z
ARG OPENSSL_VERSION=3.5.7-1~deb13u3
ARG CA_CERTIFICATES_VERSION=20250419+deb13u1
ARG NGINX_VERSION=1.30.7-1~trixie
ARG POSTGRES_META_VERSION=17+278
ARG POSTGRES_SERVER_VERSION=17.12-0+deb13u1
ARG TZDATA_VERSION=2026f-0+deb13u1
ARG ZLIB_VERSION=1:1.3.dfsg+really1.3.1-1+deb13u1
ARG S6_OVERLAY_VERSION=3.2.3.2
ARG NGINX_SIGNING_FPR=8540A6F18833A80E9C1653A42FD21310B49F6B46
ARG NGINX_SIGNING_KEY_SHA256=55385da31d198fa6a5012d40ae98ecb272a6c4e8fffffba94719ffd3e87de37a
ARG NGINX_DEB_AMD64_SHA256=7e9e787849bed1fbb677868fc8ee86f8b513cd9e46ad29e41c7710cec3c8825d
ARG NGINX_DEB_ARM64_SHA256=b32bff964b8e64e1297e6da783748b8db1117a68f3d9ad85589b41be79a38f46
ARG S6_NOARCH_SHA256=5379750ed30a84bbd2e2dd74847ba6b5bd29cd0b2e3ea2ec58049b57eb2eda12
ARG S6_AMD64_SHA256=e6befcc96a437a3831386ecfc51808c5d3e939dc5fe3c02ae9284599e8aa2408
ARG S6_ARM64_SHA256=b17f17a82e7a515c682a91edaf2ffdabb73f891981b6c1fd712115693a2f8b4c


FROM ${RUST_IMAGE} AS api-builder-os-resolved
ARG DEBIAN_SNAPSHOT
ARG DEBIAN_SECURITY_SNAPSHOT
ARG OPENSSL_VERSION
COPY docker/pins/*.sh /opt/nwui-pins/
RUN mkdir -p /nwui-record \
    && /opt/nwui-pins/base-inventory.sh > /nwui-record/base.tsv \
    && /opt/nwui-pins/configure-apt.sh "$DEBIAN_SNAPSHOT" "$DEBIAN_SECURITY_SNAPSHOT" / trixie \
    && /opt/nwui-pins/update.sh api-builder \
    && /opt/nwui-pins/install.sh /nwui-record/transaction.txt \
       build-essential pkg-config cmake "openssl=$OPENSSL_VERSION" "libssl3t64=$OPENSSL_VERSION" "openssl-provider-legacy=$OPENSSL_VERSION" "libssl-dev=$OPENSSL_VERSION" \
    && /opt/nwui-pins/inventory.sh api-builder /nwui-record/api-builder.tsv /nwui-record/base.tsv /nwui-record/transaction.txt \
    && rm -rf /var/lib/apt/lists/*

FROM scratch AS inventory-api-builder
COPY --from=api-builder-os-resolved /nwui-record/api-builder.tsv /api-builder.tsv

FROM api-builder-os-resolved AS api-builder-os
ARG TARGETARCH
COPY docker/pins/inventory/api-builder-${TARGETARCH}.tsv /nwui-expected.tsv
RUN /opt/nwui-pins/verify-inventory.sh api-builder /nwui-record/api-builder.tsv /nwui-expected.tsv \
    && rm -rf /opt/nwui-pins /nwui-expected.tsv

FROM api-builder-os AS api-builder
ARG TARGETARCH
WORKDIR /build
COPY apps/api/Cargo.toml apps/api/Cargo.lock ./apps/api/
COPY apps/api/src/ ./apps/api/src/
COPY docker/nonroot/protected-paths ./docker/nonroot/protected-paths
COPY apps/web/package.json ./apps/web/package.json
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,id=nwui-api-target-${TARGETARCH},target=/build/apps/api/target,sharing=locked \
    case "$TARGETARCH" in amd64) triple=x86_64-unknown-linux-gnu ;; arm64) triple=aarch64-unknown-linux-gnu ;; *) exit 1 ;; esac \
    && cargo fetch --locked --manifest-path apps/api/Cargo.toml --target "$triple" \
    && CARGO_BUILD_JOBS=2 cargo build --locked --release --manifest-path apps/api/Cargo.toml --bin nwui-api \
    && cp apps/api/target/release/nwui-api /build/nwui-api \
    && cargo tree --manifest-path apps/api/Cargo.toml --locked --offline -e normal,build --target "$triple" --prefix indent --charset ascii --no-dedupe > /build/cargo-tree.txt \
    && cargo metadata --manifest-path apps/api/Cargo.toml --locked --offline --format-version 1 --filter-platform "$triple" > /build/cargo-metadata.json

FROM ${NODE_IMAGE} AS base
RUN corepack enable
WORKDIR /app

FROM base AS pruner
COPY docker/turbo/ /opt/nwui-turbo/
RUN cd /opt/nwui-turbo && npm ci --ignore-scripts --no-audit --no-fund
ENV PATH="/opt/nwui-turbo/node_modules/.bin:$PATH"
COPY . .
RUN node docker/licenses/generate.mjs --check \
    && node docker/pins/check-locks.mjs /app /opt/nwui-turbo/node_modules/turbo/package.json \
    && turbo prune web --docker

FROM base AS builder
COPY --from=pruner /app/out/json/ ./
COPY --from=pruner /app/.yarnrc.yml ./.yarnrc.yml
RUN yarn install --immutable
COPY --from=pruner /app/out/full/ ./
ARG GIT_SHA=""
ENV GIT_SHA=$GIT_SHA
# images.unoptimized returns 404 before getSharp; image-optimizer/transform.js loads sharp lazily.
RUN yarn workspace web run build \
    && rm -rf /app/apps/web/.next/standalone/node_modules/sharp /app/apps/web/.next/standalone/node_modules/@img \
    && remaining="$(find /app/apps/web/.next/standalone \( -path '*/node_modules/sharp' -o -path '*/node_modules/@img' \) -print -quit)" \
    && if [ -n "$remaining" ]; then printf 'Standalone still contains sharp/@img: %s\n' "$remaining" >&2; exit 1; fi

FROM ${NODE_IMAGE} AS keyring-os-resolved
ARG DEBIAN_SNAPSHOT
ARG DEBIAN_SECURITY_SNAPSHOT
ARG OPENSSL_VERSION
ARG CA_CERTIFICATES_VERSION
COPY docker/pins/*.sh /opt/nwui-pins/
RUN mkdir -p /nwui-record \
    && /opt/nwui-pins/base-inventory.sh > /nwui-record/base.tsv \
    && /opt/nwui-pins/configure-apt.sh "$DEBIAN_SNAPSHOT" "$DEBIAN_SECURITY_SNAPSHOT" / trixie \
    && /opt/nwui-pins/update.sh keyring \
    && /opt/nwui-pins/install.sh /nwui-record/transaction.txt \
       curl gnupg "ca-certificates=$CA_CERTIFICATES_VERSION" "openssl=$OPENSSL_VERSION" "libssl3t64=$OPENSSL_VERSION" "openssl-provider-legacy=$OPENSSL_VERSION" \
    && /opt/nwui-pins/inventory.sh keyring /nwui-record/keyring.tsv /nwui-record/base.tsv /nwui-record/transaction.txt \
    && rm -rf /var/lib/apt/lists/*

FROM scratch AS inventory-keyring
COPY --from=keyring-os-resolved /nwui-record/keyring.tsv /keyring.tsv

FROM keyring-os-resolved AS keyring-os
ARG TARGETARCH
COPY docker/pins/inventory/keyring-${TARGETARCH}.tsv /nwui-expected.tsv
RUN /opt/nwui-pins/verify-inventory.sh keyring /nwui-record/keyring.tsv /nwui-expected.tsv \
    && rm -rf /opt/nwui-pins /nwui-expected.tsv

FROM keyring-os AS nginx-keyring
ARG NGINX_SIGNING_FPR
ARG NGINX_SIGNING_KEY_SHA256
RUN --mount=type=bind,source=docker/pins,target=/verify-pins,readonly \
    /verify-pins/fetch.sh keyring https://nginx.org/keys/nginx_signing.key /tmp/nginx.asc "$NGINX_SIGNING_KEY_SHA256" \
    && h=$(mktemp -d) \
    && gpg --batch --homedir "$h" --import /tmp/nginx.asc \
    && gpg --batch --homedir "$h" --export "$NGINX_SIGNING_FPR" > /nginx-archive-keyring.gpg \
    && test -s /nginx-archive-keyring.gpg \
    && test "$(gpg --batch --show-keys --with-colons /nginx-archive-keyring.gpg | awk -F: '$1=="fpr" {print $10;exit}')" = "$NGINX_SIGNING_FPR" \
    && rm -rf "$h" /tmp/nginx.asc

FROM nginx-keyring AS s6-download
ARG TARGETARCH
ARG S6_OVERLAY_VERSION
ARG S6_NOARCH_SHA256
ARG S6_AMD64_SHA256
ARG S6_ARM64_SHA256
RUN --mount=type=bind,source=docker/pins,target=/verify-pins,readonly \
    case "$TARGETARCH" in \
      amd64) s6_arch=x86_64; expected="$S6_AMD64_SHA256" ;; \
      arm64) s6_arch=aarch64; expected="$S6_ARM64_SHA256" ;; \
      *) exit 1 ;; \
    esac \
    && /verify-pins/fetch.sh s6 "https://github.com/just-containers/s6-overlay/releases/download/v${S6_OVERLAY_VERSION}/s6-overlay-noarch.tar.xz" /s6-overlay-noarch.tar.xz "$S6_NOARCH_SHA256" \
    && /verify-pins/fetch.sh s6 "https://github.com/just-containers/s6-overlay/releases/download/v${S6_OVERLAY_VERSION}/s6-overlay-${s6_arch}.tar.xz" /s6-overlay-arch.tar.xz "$expected"

FROM ${NODE_IMAGE} AS runtime-os-resolved
ARG DEBIAN_SNAPSHOT
ARG DEBIAN_SECURITY_SNAPSHOT
ARG OPENSSL_VERSION
ARG CA_CERTIFICATES_VERSION
ARG NGINX_VERSION
ARG POSTGRES_META_VERSION
ARG POSTGRES_SERVER_VERSION
ARG TZDATA_VERSION
ARG ZLIB_VERSION
ARG NGINX_DEB_AMD64_SHA256
ARG NGINX_DEB_ARM64_SHA256
ARG TARGETARCH
COPY docker/pins/*.sh /opt/nwui-pins/
COPY --from=nginx-keyring /nginx-archive-keyring.gpg /usr/share/keyrings/nginx-archive-keyring.gpg
# Build-stage package managers remain available; remove them in runtime only.
RUN rm -rf /usr/local/lib/node_modules/npm /usr/local/lib/node_modules/corepack \
           /opt/yarn-v* /usr/local/bin/npm /usr/local/bin/npx \
           /usr/local/bin/corepack /usr/local/bin/yarn /usr/local/bin/yarnpkg \
    && for path in /usr/local/lib/node_modules/npm /usr/local/lib/node_modules/corepack \
                   /opt/yarn-v* /usr/local/bin/npm /usr/local/bin/npx /usr/local/bin/corepack /usr/local/bin/yarn /usr/local/bin/yarnpkg; do \
         if [ -e "$path" ] || [ -L "$path" ]; then echo "Runtime package manager remains: $path" >&2; exit 1; fi; \
       done \
    && mkdir -p /nwui-record \
    && /opt/nwui-pins/base-inventory.sh > /nwui-record/base.tsv \
    && /opt/nwui-pins/configure-apt.sh "$DEBIAN_SNAPSHOT" "$DEBIAN_SECURITY_SNAPSHOT" / trixie \
    && /opt/nwui-pins/update.sh runtime \
    && printf '#!/bin/sh\nexit 101\n' > /usr/sbin/policy-rc.d \
    && chmod 755 /usr/sbin/policy-rc.d \
    && /opt/nwui-pins/install.sh /nwui-record/bootstrap.txt \
         "ca-certificates=$CA_CERTIFICATES_VERSION" "openssl=$OPENSSL_VERSION" "libssl3t64=$OPENSSL_VERSION" "openssl-provider-legacy=$OPENSSL_VERSION" "zlib1g=$ZLIB_VERSION" postgresql-common \
    && printf '\ncreate_main_cluster = false\n' >> /etc/postgresql-common/createcluster.conf \
    && printf '%s\n' 'deb [signed-by=/usr/share/keyrings/nginx-archive-keyring.gpg] https://nginx.org/packages/debian trixie nginx' > /etc/apt/sources.list.d/nginx.list \
    && /opt/nwui-pins/update.sh runtime \
    && cd /tmp \
    && /opt/nwui-pins/download-deb.sh runtime "nginx=$NGINX_VERSION" \
    && case "$TARGETARCH" in amd64) expected="$NGINX_DEB_AMD64_SHA256" ;; arm64) expected="$NGINX_DEB_ARM64_SHA256" ;; *) exit 1 ;; esac \
    && /opt/nwui-pins/verify-file.sh runtime "/tmp/nginx_${NGINX_VERSION}_${TARGETARCH}.deb" "$expected" \
    && printf '%s\n' \
         'path-exclude=/usr/lib/*-linux-gnu/libLLVM*.so*' \
         'path-exclude=/usr/lib/*-linux-gnu/libz3.so*' \
         'path-exclude=/usr/lib/postgresql/17/lib/llvmjit*' \
         'path-exclude=/usr/lib/postgresql/17/lib/bitcode/*' \
         > /etc/dpkg/dpkg.cfg.d/nwui-no-jit \
    && /opt/nwui-pins/install.sh /nwui-record/runtime.txt \
         "nginx=$NGINX_VERSION" "postgresql=$POSTGRES_META_VERSION" \
         "postgresql-17=$POSTGRES_SERVER_VERSION" "postgresql-client-17=$POSTGRES_SERVER_VERSION" "tzdata=$TZDATA_VERSION" xz-utils \
    && /opt/nwui-pins/inventory.sh runtime /nwui-record/runtime.tsv /nwui-record/base.tsv /nwui-record/bootstrap.txt /nwui-record/runtime.txt \
    && dpkg-query -W -f='${binary:Package}\t${db:Status-Status}\n' apt libapt-pkg7.0 > /tmp/nwui-apt-packages \
    && awk -F '\t' '{sub(/:[^:]*$/, "", $1); if ($2!="installed") exit 1; seen[$1]++} END {if (seen["apt"]!=1 || seen["libapt-pkg7.0"]!=1) exit 1}' /tmp/nwui-apt-packages \
    && awk -F '\t' '$1=="apt" || $1=="libapt-pkg7.0" {seen[$1]++} END {if (seen["apt"]!=1 || seen["libapt-pkg7.0"]!=1) exit 1}' /nwui-record/runtime.tsv \
    && dpkg-query -L apt libapt-pkg7.0 > /tmp/nwui-apt-paths \
    && : > /tmp/nwui-apt-payload \
    && while IFS= read -r f; do \
         if [ -f "$f" ] || [ -L "$f" ]; then printf '%s\n' "$f" >> /tmp/nwui-apt-payload; fi; \
       done < /tmp/nwui-apt-paths \
    && dpkg --purge apt libapt-pkg7.0 \
    && while IFS= read -r f; do \
         if [ -e "$f" ] || [ -L "$f" ]; then echo "Removed apt payload remains: $f" >&2; exit 1; fi; \
       done < /tmp/nwui-apt-payload \
    && dpkg --audit > /tmp/nwui-apt-audit \
    && [ ! -s /tmp/nwui-apt-audit ] \
    && awk -F '\t' '$1=="apt" || $1=="libapt-pkg7.0" {removed++; next} {print} END {if (removed!=2) exit 1}' /nwui-record/runtime.tsv > /tmp/nwui-post-apt.tsv \
    && /bin/bash -o pipefail /opt/nwui-pins/base-inventory.sh > /tmp/nwui-post-apt-installed.tsv \
    && awk -F '\t' 'BEGIN {OFS="\t"} NR>1 {print $1,$2,$3,$4,$5}' /tmp/nwui-post-apt.tsv > /tmp/nwui-post-apt-expected.tsv \
    && cmp /tmp/nwui-post-apt-installed.tsv /tmp/nwui-post-apt-expected.tsv \
    && mv /tmp/nwui-post-apt.tsv /nwui-record/runtime.tsv \
    && /opt/nwui-pins/verify-no-apt.sh \
    && rm -f /tmp/nwui-apt-packages /tmp/nwui-apt-paths /tmp/nwui-apt-payload /tmp/nwui-apt-audit /tmp/nwui-post-apt-installed.tsv /tmp/nwui-post-apt-expected.tsv \
    && for f in /usr/lib/*/libLLVM* /usr/lib/*/libz3.so* /usr/lib/postgresql/*/lib/llvmjit*; do \
         if [ -e "$f" ] || [ -L "$f" ]; then echo "PostgreSQL JIT file shipped: $f" >&2; exit 1; fi; \
       done \
    && if [ -n "$(find /usr/lib/postgresql -path '*/bitcode/*' -print -quit)" ]; then echo "PostgreSQL JIT bitcode shipped" >&2; exit 1; fi \
    && for f in /usr/lib/postgresql/*/lib/*.so /usr/lib/postgresql/*/bin/*; do \
         if ldd "$f" 2>/dev/null | grep -q 'not found'; then echo "PostgreSQL file with a missing library: $f" >&2; exit 1; fi; \
       done \
    && rm -f /etc/nginx/conf.d/default.conf /tmp/nginx_*.deb \
    && rm -rf /var/lib/apt/lists/*

FROM scratch AS inventory-runtime
COPY --from=runtime-os-resolved /nwui-record/runtime.tsv /runtime.tsv

FROM runtime-os-resolved AS runtime-os
ARG TARGETARCH
COPY docker/pins/inventory/runtime-${TARGETARCH}.tsv /nwui-expected.tsv
RUN /opt/nwui-pins/verify-inventory.sh runtime /nwui-record/runtime.tsv /nwui-expected.tsv \
    && rm -rf /opt/nwui-pins /nwui-expected.tsv

FROM runtime-os AS runtime-content
ARG NODE_IMAGE
ARG RUST_IMAGE
ARG S6_OVERLAY_VERSION
ARG S6_NOARCH_SHA256
ARG S6_AMD64_SHA256
ARG S6_ARM64_SHA256
ARG NGINX_SIGNING_KEY_SHA256
ARG GIT_SHA=""
ARG SOURCE_TREE=""
LABEL org.opencontainers.image.source="https://github.com/xelexis-dev/nginx-web-ui" \
      org.opencontainers.image.url="https://github.com/xelexis-dev/nginx-web-ui" \
      org.nginx-web-ui.source-tree="${SOURCE_TREE}"
ENV APP_GIT_SHA=$GIT_SHA
COPY --from=api-builder /build/nwui-api /usr/local/bin/nwui-api
LABEL org.nginx-web-ui.distro="trixie" \
      org.nginx-web-ui.nginx-version="${NGINX_VERSION}" \
      org.nginx-web-ui.postgres-version="${POSTGRES_SERVER_VERSION}" \
      org.nginx-web-ui.tzdata-version="${TZDATA_VERSION}" \
      org.nginx-web-ui.zlib-version="${ZLIB_VERSION}" \
      org.nginx-web-ui.s6-version="${S6_OVERLAY_VERSION}" \
      org.nginx-web-ui.node-image="${NODE_IMAGE}" \
      org.nginx-web-ui.debian-snapshot="${DEBIAN_SNAPSHOT}" \
      org.nginx-web-ui.debian-security-snapshot="${DEBIAN_SECURITY_SNAPSHOT}" \
      org.nginx-web-ui.openssl-version="${OPENSSL_VERSION}"
RUN --mount=type=bind,from=s6-download,source=/s6-overlay-noarch.tar.xz,target=/tmp/s6-overlay-noarch.tar.xz \
    --mount=type=bind,from=s6-download,source=/s6-overlay-arch.tar.xz,target=/tmp/s6-overlay-arch.tar.xz \
    tar -C / -Jxpf /tmp/s6-overlay-noarch.tar.xz && tar -C / -Jxpf /tmp/s6-overlay-arch.tar.xz
RUN ! getent passwd 10001 && ! getent group 10001 \
 && groupadd -r -g 10001 nwui-api \
 && useradd -r -u 10001 -g 10001 -G nginx,adm -d /nonexistent -s /usr/sbin/nologin nwui-api
WORKDIR /app
COPY --from=builder /app/apps/web/.next/standalone ./
COPY --from=builder /app/apps/web/.next/static ./apps/web/.next/static
COPY --from=builder /app/apps/web/public ./apps/web/public
# Next's standalone tracer copies the ORIGINAL .ts/.tsx of traced server files
# (server actions, route handlers, lib/*, instrumentation.ts, proxy.ts) into the
# image even though node only ever executes the compiled .next/server output.
# Strip that dead source so a puller (pull is anonymous) can't read the verbatim
# server TypeScript. Runtime is unaffected — `node apps/web/server.js` loads
# .next/server/*.js, never these .ts. NOTE: scoped to apps/web; the drizzle SQL
# copied below is load-bearing (migrations run from it at boot) and is untouched.
RUN find /app/apps/web -type f \( -name '*.ts' -o -name '*.tsx' \) -delete
# The Rust API reads the journal and SQL from MIGRATIONS_DIR.
COPY packages/db/drizzle/*.sql /app/drizzle/
COPY packages/db/drizzle/meta/_journal.json /app/drizzle/meta/_journal.json
# 사용자용 변경 내역. 서버에서 "지금 도는 이미지가 뭘 바꿨나"를 이미지만 보고
# 확인할 수 있게 넣는다 — 저장소를 받지 않고도 교차 확인이 되고, 여러 버전을
# 건너뛰어 올릴 때 챙길 것을 그 자리에서 읽을 수 있다.
COPY UPDATE.md /app/UPDATE.md
COPY LICENSE NOTICE THIRD_PARTY_NOTICES.md /usr/share/nginx-web-ui/licenses/
COPY docker/nginx.conf /etc/nginx/nginx.conf
COPY docker/conf.d/ /etc/nginx/conf.d/
COPY docker/stream.d/ /etc/nginx/stream.d/
COPY docker/snippets/ /etc/nginx/snippets/
COPY docker/s6-rc.d/ /etc/s6-overlay/s6-rc.d/
# 어떤 서비스를 올릴지 고르는 `user` 번들은 **s6-rc.d 밖**에 둔다. s6-overlay 3.2.3
# 부터 s6-rc.d 안에 user 번들을 두는 것이 deprecated 라, 두면 부팅할 때마다 경고가
# 찍힌다(사용자가 `docker logs` 에서 그대로 본다).
#
# s6-overlay 의 `top` 번들이 `user` 와 `user2` 를 **둘 다** 요구해서, 쓰지 않는
# user2 도 빈 번들로 있어야 컴파일이 통과한다. 빈 디렉터리는 git 이 보관하지
# 못하고, 자리표시 파일을 넣으면 s6-rc-compile 이 그 이름을 서비스로 찾다가
# 실패하므로 여기서 만든다.
COPY docker/user-bundles.d/ /etc/s6-overlay/user-bundles.d/
RUN mkdir -p /etc/s6-overlay/user-bundles.d/user2/contents.d
COPY docker/scripts/ /etc/s6-overlay/scripts/
COPY docker/nonroot/protected-paths /etc/s6-overlay/nonroot/protected-paths
RUN chmod -R go-w /etc/s6-overlay
COPY --chmod=755 docker/nonroot/api-pre-exec /usr/local/bin/api-pre-exec
COPY docker/upgrade-status/ /usr/share/nginx-web-ui/upgrade-status/
COPY packages/ui/src/tokens.css /usr/share/nginx-web-ui/upgrade-status/tokens.css
COPY packages/ui/src/button.module.css /usr/share/nginx-web-ui/upgrade-status/button.css
COPY --from=builder /app/node_modules/pretendard/dist/web/variable/pretendardvariable-dynamic-subset.css /usr/share/nginx-web-ui/upgrade-status/fonts/pretendardvariable-dynamic-subset.css
COPY --from=builder /app/node_modules/pretendard/dist/web/variable/woff2-dynamic-subset/ /usr/share/nginx-web-ui/upgrade-status/fonts/woff2-dynamic-subset/
COPY --chmod=755 docker/scripts/nwui-db-upgrade.sh /usr/local/bin/nwui-db-upgrade
# Reference copies for the operator — NOT active inside the container. fail2ban
# runs on the host and reads the log volumes below; see the README in there for
# why the app writes its own auth log and for the DOCKER-USER chain caveat.
COPY docker/fail2ban/ /usr/share/nginx-web-ui/fail2ban/
# Validate every shipped nginx include without creating deployment TLS material.
# The boot-rewritten upstream include already ships valid loopback defaults.
RUN set -eu; \
    nginx_check_dir="$(mktemp -d /tmp/nwui-nginx-check.XXXXXX)"; \
    trap 'rm -rf "$nginx_check_dir"' EXIT; \
    openssl req -x509 -newkey rsa:2048 -nodes -days 1 \
      -subj '/CN=build-check.invalid' \
      -keyout "$nginx_check_dir/default.key" -out "$nginx_check_dir/default.crt" >/dev/null 2>&1; \
    sed -e "s|/var/lib/nginx-web-ui/tls/default\.crt|$nginx_check_dir/default.crt|g" \
        -e "s|/var/lib/nginx-web-ui/tls/default\.key|$nginx_check_dir/default.key|g" \
        /etc/nginx/nginx.conf > "$nginx_check_dir/nginx.conf"; \
    nginx -t -c "$nginx_check_dir/nginx.conf"
RUN chmod -R +x /etc/s6-overlay/scripts /etc/s6-overlay/s6-rc.d/postgres/run /etc/s6-overlay/s6-rc.d/nginx/run /etc/s6-overlay/s6-rc.d/web/run /etc/s6-overlay/s6-rc.d/api/run /etc/s6-overlay/s6-rc.d/dns-guard/run /etc/s6-overlay/s6-rc.d/upgrade-runner/run /etc/s6-overlay/s6-rc.d/storage-locks/run /etc/s6-overlay/s6-rc.d/upgrade-status/run /etc/s6-overlay/s6-rc.d/publisher/run
# Archive modes are not an authority to make root-consumed inputs writable.
RUN set -eu; \
    set -- /etc/nginx/nginx.conf /etc/nginx/conf.d /etc/nginx/stream.d /etc/nginx/snippets \
        /app/drizzle /usr/local/bin/nwui-api /usr/local/bin/api-pre-exec /usr/local/bin/nwui-db-upgrade; \
    chmod -R go-w "$@"; \
    writable="$(find /etc/s6-overlay "$@" -perm /022 -print -quit)"; \
    if [ -n "$writable" ]; then printf 'Group/world writable trusted root input: %s\n' "$writable" >&2; exit 1; fi

ENV PORT=3000 API_PORT=3001 NODE_ENV=production PGROOT=/var/lib/postgresql/data PGDATA=/var/lib/postgresql/data/17/data DATABASE_URL=postgres://nginx:nginx@127.0.0.1:5432/nginx_web_ui
ENV MIGRATIONS_DIR=/app/drizzle
# Failed boot prerequisites must exit so the operator's restart policy can act.
ENV S6_BEHAVIOUR_IF_STAGE2_FAILS=2
# Loopback is this container: proxy/stream targets on the web app, API or database ports are
# refused (see control/internal_targets.rs). Outside the image loopback is someone else's.
ENV NWUI_CONTAINER=1
# AUTH_SECRET is generated at first boot unless supplied. ADMIN_PASSWORD is
# optional: supply it with ADMIN_EMAIL, or create the first admin in /setup.
# The DB password is also provisioned by secrets-init: fresh volumes get a random
# per-deploy password (persisted to /var/lib/nginx-web-ui/secrets/db_password) and
# DATABASE_URL below is rebuilt from it at boot; existing volumes keep 'nginx'
# (backward-compatible, never rotated). The baked DATABASE_URL is only a fallback.
ENV ADMIN_EMAIL=admin@example.com
# Rust validates proxy trust and browser origins; Next no longer authenticates.
EXPOSE 8080 80 443
# Rust writes the app's own sign-in-failure log (auth/auth_log.rs).
# Created here so the first write never races on mkdir, and so the mount point
# exists even when an operator bind-mounts an empty host directory over it.
RUN mkdir -p /var/log/nginx-web-ui /app/apps/web/.next/cache \
    && chown -R node:node /app/apps/web/.next/cache \
    && chmod 0700 /app/apps/web/.next/cache
# Both log directories are volumes so a host-side fail2ban can read them and so
# logs survive container recreation — before this they lived only in the
# container layer and were unreadable from the host, which is exactly what an
# operational security review flagged. NOTE: persisted logs now grow without
# bound; set up host logrotate (docker/fail2ban/README.md has a sample).
VOLUME ["/var/lib/postgresql/data", "/var/lib/nginx-web-ui", "/var/log/nginx", "/var/log/nginx-web-ui"]
# The API binary checks nginx -> API -> database and the SSR process (see healthcheck.rs).
# A Node script did the same but started a Node runtime every 30 s, most of the idle CPU.
HEALTHCHECK --interval=30s --timeout=10s --start-period=900s --retries=3 \
    CMD ["/usr/local/bin/nwui-api", "healthcheck"]
ENTRYPOINT ["/init"]

# Export the complete record first; the checked runner cannot accept an empty or
# invented label. The release wrapper builds this target, hashes it, then builds
# runner with that hash (the content layers are cached, not rebuilt).
RUN --mount=type=bind,from=api-builder,source=/build,target=/nwui-api-build \
    --mount=type=bind,from=api-builder-os,source=/nwui-record,target=/nwui-api-builder-record \
    --mount=type=bind,from=keyring-os,source=/nwui-record,target=/nwui-keyring-record \
    --mount=type=bind,source=docker/pins,target=/build-pins,readonly \
    --mount=type=bind,source=.,target=/input,readonly \
    mkdir -p /tmp/nwui-input /tmp/nwui-record \
    && cp /nwui-api-build/cargo-tree.txt /nwui-api-build/cargo-metadata.json /tmp/nwui-record/ \
    && cp /nwui-api-builder-record/api-builder.tsv /tmp/nwui-record/api-builder.tsv \
    && cp /nwui-keyring-record/keyring.tsv /tmp/nwui-record/keyring.tsv \
    && cp /input/Dockerfile /input/package.json /input/yarn.lock /tmp/nwui-input/ \
    && cp /input/apps/web/package.json /tmp/nwui-input/web-package.json \
    && cp /input/apps/api/Cargo.lock /tmp/nwui-input/Cargo.lock \
    && cp /input/docker/turbo/package-lock.json /tmp/nwui-input/package-lock.json \
    && cp /nwui-record/runtime.tsv /tmp/nwui-record/runtime.tsv \
    && cp /nwui-record/runtime.tsv /app/dpkg-inventory.tsv \
    && node /build-pins/build-inputs.mjs /build-pins /tmp/nwui-input /tmp/nwui-record /app/build-inputs.json \
    && rm -rf /nwui-record /tmp/nwui-record /tmp/nwui-input

FROM scratch AS build-inputs
COPY --from=runtime-content /app/build-inputs.json /app/dpkg-inventory.tsv /

FROM runtime-content AS runner
ARG INPUTS_SHA256
RUN --mount=type=bind,source=docker/pins,target=/verify-pins,readonly \
    /verify-pins/verify-file.sh runtime /app/build-inputs.json "$INPUTS_SHA256" \
    && /verify-pins/verify-no-apt.sh
# Remove unused privilege bits and retain only the required helpers.
RUN set -eu; \
    set --; \
    for h1_tool in mount umount su passwd chfn chsh gpasswd newgrp chage expiry; do \
        h1_path=$(command -v "$h1_tool") && h1_path=$(readlink -f "$h1_path") && [ -f "$h1_path" ] \
            || { echo 'H1: cannot resolve a required runtime tool' >&2; exit 1; }; \
        set -- "$@" "$h1_path"; \
    done; \
    chmod u-s,g-s "$@"; \
    h1_unix=$(readlink -f /usr/sbin/unix_chkpwd) && h1_suexec=$(readlink -f /command/s6-overlay-suexec) \
        && [ -f "$h1_unix" ] && [ -f "$h1_suexec" ] && [ "$h1_unix" != "$h1_suexec" ] \
        || { echo 'H1: required allowlist helper is missing' >&2; exit 1; }; \
    h1_expected=$(printf '%s\n' "$h1_unix" "$h1_suexec" | LC_ALL=C sort); \
    h1_actual=$(find / -xdev -type f -perm /6000 -print) \
        || { echo 'H1: unable to enumerate privileged files' >&2; exit 1; }; \
    h1_actual=$(printf '%s\n' "$h1_actual" | LC_ALL=C sort); \
    [ "$h1_actual" = "$h1_expected" ] \
        || { echo 'H1: setuid/setgid files differ from the two-helper allowlist' >&2; printf '%s\n' "$h1_actual" >&2; exit 1; }
LABEL org.nginx-web-ui.inputs-sha256="${INPUTS_SHA256}"
