CREATE TABLE "certificate_jobs" (
	"id" text PRIMARY KEY NOT NULL,
	"certificate_id" text NOT NULL,
	"kind" text NOT NULL,
	"mode" text NOT NULL,
	"state" text DEFAULT 'queued' NOT NULL,
	"cancel_requested" boolean DEFAULT false NOT NULL,
	"payload_enc" text NOT NULL,
	"actor" jsonb NOT NULL,
	"http_tokens" jsonb DEFAULT '[]'::jsonb NOT NULL,
	"cf_records" jsonb DEFAULT '[]'::jsonb NOT NULL,
	"error" text,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "certificate_jobs_kind" CHECK ("certificate_jobs"."kind" IN ('http','dns_begin','dns_complete','dns_auto')),
	CONSTRAINT "certificate_jobs_mode" CHECK ("certificate_jobs"."mode" IN ('initial','renew')),
	CONSTRAINT "certificate_jobs_state" CHECK ("certificate_jobs"."state" IN ('queued','active','cleanup','succeeded','failed','cancelled'))
);
--> statement-breakpoint
CREATE UNIQUE INDEX "certificate_jobs_one_active" ON "certificate_jobs" USING btree ("certificate_id") WHERE "certificate_jobs"."state" IN ('queued','active','cleanup');