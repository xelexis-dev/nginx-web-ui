ALTER TABLE audit_logs ADD COLUMN via jsonb NULL;
--> statement-breakpoint
UPDATE access_tokens SET revoked_at = now() WHERE kind = 'user' AND revoked_at IS NULL;
