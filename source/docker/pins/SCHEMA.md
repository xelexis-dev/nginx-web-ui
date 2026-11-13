# Build-input records v1

`inventory/<stage>-<arch>.tsv` uses UTF-8, LF, an exact header followed by rows
sorted bytewise (`LC_ALL=C`) by all six tab-separated columns:

```
binary	arch	version	source	sourceVersion	origin
```

Here `\t` denotes a literal tab. No field contains tabs or newlines. One row per
installed binary package (`install ok installed`), no duplicate `(binary,arch)`.
`binary` excludes a dpkg multiarch `:arch` suffix; `arch` is the **package**
architecture (`all`, `amd64`, `arm64`). The filename architecture is the target
architecture. `source`/`sourceVersion` are dpkg's source package identity, including
fallback to the binary package/version when the source field is absent.
`inventory-row.schema.json` describes the object produced by a TSV reader.

Stages: `api-builder`, `keyring`, `runtime`. Only their OS dependencies are in
these inventories; JS/Rust/tool artifacts have separate lock/integrity records.
`origin` is `base` for an unchanged base row; otherwise apt's signed configured
index/transaction identifies `debian`, `debian-security`, `nginx.org`, or `pgdg` for exactly the two reviewed migration-only PG15 packages.
It is never inferred from dpkg alone. If origin is ambiguous, generation fails.

The runtime installed set is recorded after purging only `apt` and `libapt-pkg7.0`.
Retained origins are proved before purge; all five retained package identity columns
are checked against the actual installed set afterward. Builder inventories retain apt.

Normal `*-os` stages compare all six columns against committed inventories.
Dedicated scratch `inventory-<stage>` targets export resolved inventories for
an explicit refresh; no build argument can disable normal enforcement.
The 1.19.11 reproduction gate compares only the first five normalized columns.

The runtime contains `/app/dpkg-inventory.tsv` and `/app/build-inputs.json`.
The latter follows `build-inputs.schema.json`, schemaVersion 1. Hashes are lowercase
SHA-256 of the exact corresponding file bytes (including TSV header/final LF).
Base image `image` includes the pinned index digest; `childDigest` identifies the
selected platform manifest. `integrity.frontend` is a tag@sha256 reference.
Each `inventories.<stage>.sha256` identifies the checked per-target inventory.
`nodeVersions` is Node's actual `process.versions`, including its own OpenSSL.

`crateGraph.kind` is `build-graph`, **not proved linkage**. `treeSha256` identifies
`cargo tree --locked --offline -e normal,build --target <triple> --prefix indent --charset ascii --no-dedupe`
output taken after the build. `normal`/`build` are sorted distinct `name@version`
identities from actual tree branches and explicit `[build-dependencies]` scopes.
Build ancestry applies transitively; a shared crate can occur in both sets.
`lockedOnly` preserves every other Cargo.lock identity, including optional, dev
and other-platform packages. The tree includes its root as normal. Its union is
a subset of the lock; tree union lockedOnly equals the complete lock exactly,
and lockedOnly is disjoint from normal/build. Metadata feature unification does
not determine membership or classification. Unsupported or deduplicated output
fails closed.
Do not label them linked without separate binary-linkage evidence.

No private source hosts, credentials or checkout paths belong in either record.
Build infrastructure versions are recorded separately in job evidence; they are
not reproducible product inputs managed by this repository.

Trixie records additionally carry `directPins.pg15Helper` and the two per-architecture `integrity.pg15ServerSha256`/`pg15ClientSha256` artifact hashes. These optional v1 fields preserve readability of prior Bookworm records; the Trixie Dockerfile always supplies them. PGDG is never configured as an apt source; its two verified local artifacts have explicit transaction provenance, with dependencies resolved exclusively from signed Debian snapshots.
