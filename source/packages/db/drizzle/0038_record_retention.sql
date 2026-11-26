ALTER TABLE app_settings
    ADD COLUMN audit_retention_days integer CHECK (audit_retention_days BETWEEN 90 AND 3650),
    ADD COLUMN history_retention_days integer CHECK (history_retention_days BETWEEN 30 AND 3650);
--> statement-breakpoint
CREATE TABLE retention_state (
    collection text PRIMARY KEY CHECK (collection IN ('audit', 'history')),
    next_run_at timestamptz,
    run_started_at timestamptz,
    first_run_pending boolean NOT NULL DEFAULT false,
    last_run_at timestamptz,
    last_run_count bigint NOT NULL DEFAULT 0,
    last_run_cutoff timestamptz,
    last_cleanup_at timestamptz,
    last_cleanup_count bigint NOT NULL DEFAULT 0,
    total_deleted bigint NOT NULL DEFAULT 0,
    unreceipted_deleted bigint NOT NULL DEFAULT 0,
    pending_days integer,
    pending_cutoff timestamptz,
    failure_at timestamptz,
    failure_message text CHECK (char_length(failure_message) <= 500 AND failure_message !~ '[[:cntrl:]]'),
    failure_count bigint NOT NULL DEFAULT 0,
    newest_deleted_at timestamptz,
    newest_deleted_id text
);
--> statement-breakpoint
INSERT INTO retention_state (collection) VALUES ('audit'), ('history');
