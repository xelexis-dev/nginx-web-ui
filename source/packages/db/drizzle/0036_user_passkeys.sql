ALTER TABLE "users" ADD COLUMN "passkey_user_handle" bytea UNIQUE;
--> statement-breakpoint
CREATE TABLE "user_passkeys" (
  "id" text PRIMARY KEY,
  "user_id" text NOT NULL REFERENCES "users"("id") ON DELETE CASCADE,
  "credential_id" bytea NOT NULL UNIQUE,
  "public_key" bytea NOT NULL,
  "algorithm" integer NOT NULL CHECK ("algorithm" IN (-7, -257)),
  "rp_id" text NOT NULL,
  "origin" text NOT NULL,
  "sign_count" bigint NOT NULL DEFAULT 0 CHECK ("sign_count" BETWEEN 0 AND 4294967295),
  "backup_eligible" boolean NOT NULL,
  "backup_state" boolean NOT NULL,
  "transports" text[],
  "name" text NOT NULL,
  "created_at" timestamptz NOT NULL DEFAULT now(),
  "last_used_at" timestamptz
);
--> statement-breakpoint
CREATE INDEX "user_passkeys_user_id_idx" ON "user_passkeys"("user_id");
