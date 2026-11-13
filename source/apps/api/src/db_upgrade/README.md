# Database upgrade state v1

`state-v1.schema.json` describes both `nwui-db-upgrade --json` output and
`/destination/db/.nwui/migration-v1.json`. `pair-v1.schema.json` describes the
matching `.nwui/pair-v1.json` on each DB/app volume. Examples are synthetic.

The PostgreSQL identifier spelling is **`source.systemIdentifier`** and
**`target.systemIdentifier`** in status/state, and **`systemIdentifier`** at
the top level of a pair record. Identifiers are decimal strings, preserving
all 64 bits. There is no `pgSystemIdentifier` alias. PG17 has a new identifier;
compare it to the target control data, never to PG15's identifier.

`phase` records the last completed step:
`not_started`, `validated`, `copied`, `checked`, `converted`, `verified`, `ready`.
Before a mutation, persist `operation: "running"` and the intended `nextPhase`.
Idle/failed operations have `nextPhase: null`; failure has a sanitized
`lastError`. A running record after interruption never proves completion.
The engine additionally validates legal transitions and filesystem identity;
schema validity alone cannot authorize conversion, continuation or startup.

`ready: true` requires the ready phase, an idle operation, all named checks,
and the complete destination/target identity. Status exit 0 alone is insufficient.
The classifier also requires matching pair records and actual control identity.

`destinations.db` and `.app` are display metadata: `{kind: "volume", source:
"name"}` or `{kind: "bind", source: "/absolute/host/path"}`. They never supply
filesystem command arguments. Actual roots stay `/source/{db,app}` and
`/destination/{db,app}`. Target/pair architecture names are `arm64` and `amd64`. Source architecture also
allows `unrecorded`; released 1.19.x volumes always use that value because they
contain no architecture record. It is never inferred from the candidate. The
PG15 conversion guard compares binary control-data properties and runs
`pg_upgrade --check`; unknown original architecture is not proof of a match.
The operator must use the architecture that produced the data.

`checks` names are `sourceUnchanged`, `rolePassword`, `encryptedData`,
`logicalData`, `collations`, `cleanShutdown`, `pairAgreement`.
`summary.tableCounts` maps qualified table names to row counts; other summary
fields are byte counts, or null until known. Public JSON contains no keys,
passwords, SQL credentials, raw command errors or environment values.

Fresh pair intent has `mode: "fresh"`, `ready: false`, and `freshStep` of
`secrets`, `initdb`, `role`, `database`, or `verify`. Its identifier is null until
initdb completes. Migration pair records use `freshStep: null`; all ready pair
records have a nonnull identifier and a null fresh step.

The engine modules implement independent private copies, compatibility checks,
conversion and verification. Runtime dispatch and image installation are separate
integration steps. Opt-in native tests use four disposable mounted volumes;
a test-tool image is not a product candidate.

`verify` is diagnostic: a converted attempt stays not ready, with its journal
unchanged. Only an uninterrupted `run` publishes verified/ready pair records.
A repeated `run` on a ready pair validates identity/key agreement, without
comparing later business writes to the old baseline. Explicit `verify` reports
post-cutover changes on stderr as information (exit 0); JSON stdout remains the
strict State-v1 object. Failed/interrupted workspaces and source volumes remain
available for inspection; a failed conversion needs a new destination pair.

Source inspection enumerates every non-shared `pg_catalog` table with an `oid`
column. Unknown catalog classes and user OIDs in system schemas refuse. The
application exceptions are limited to objects already described in the logical
baseline: public/drizzle namespaces, known tables and their indexes/owned TOAST,
table/enum types, constraints/defaults, enum labels, and internal built-in foreign
key triggers (including their definitions and enabled state). Fresh inspection
has no application exceptions; shared database/role objects are checked separately.
Migration inspection ignores empty numbered `pg_temp_N`/`pg_toast_temp_N`
namespaces left by earlier sessions; objects inside them still refuse. The
fresh-only catalog guard also refuses these empty namespaces.

`legacy-1.19.11.json` freezes the 28 SQL hashes and journal times from released
commit `567956d954bb5f272c1640dec249903f8c58635b`. Source `check` compares the
distinct `(hash, created_at)` set on the private PG15 copy before target initdb,
conversion or ready-pair publication. Missing, unknown or null signatures refuse
with the 1.19.11-first instruction; duplicate history rows are preserved. This
source prerequisite is not reapplied to an already converted PG17 database.
It runs immediately after validating the role/database catalog, before the data
profile, and reads only a plain history table with built-in text/bigint columns.

Stop the app container before offline `check`, `run` or `verify`. The supervised
postmaster holds a shared PGROOT flock for its lifetime; these commands require
an exclusive flock and refuse with `storage_busy` while it runs. Boot key binding
uses a shared DB-root lock and an exclusive app-root lock.
