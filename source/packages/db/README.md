# SQL migrations

Migrations are hand-written SQL in `drizzle/<tag>.sql`, with a matching entry in
`drizzle/meta/_journal.json`. Append a unique tag, the next consecutive `idx`, and
an increasing `when`; preserve existing SQL and meta files as applied history.
This workspace stores SQL history only; it has no TypeScript runtime or schema export.

Rust owns migration execution and validation. Run the Rust migration tests
(`cargo test --locked --test migration_files` and the database migration tests),
from `apps/api`. Database tests require `NWUI_TEST_DATABASE_URL` pointing to a
disposable test database. The TypeScript migration generator has been retired.
