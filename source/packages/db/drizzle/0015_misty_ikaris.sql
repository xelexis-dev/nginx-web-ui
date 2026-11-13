CREATE TYPE "public"."webhook_format" AS ENUM('generic', 'discord', 'custom');--> statement-breakpoint
CREATE TABLE "webhook_targets" (
	"id" text PRIMARY KEY NOT NULL,
	"name" text NOT NULL,
	"enabled" boolean DEFAULT true NOT NULL,
	"format" "webhook_format" DEFAULT 'generic' NOT NULL,
	"url_enc" text NOT NULL,
	"headers_enc" text,
	"body_template" text,
	"events" jsonb NOT NULL,
	"last_attempt_at" timestamp with time zone,
	"last_success_at" timestamp with time zone,
	"last_status" integer,
	"last_error" text,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	"actor_id" text
);
--> statement-breakpoint
ALTER TABLE "app_settings" ADD COLUMN "public_base_url" text;--> statement-breakpoint
ALTER TABLE "webhook_targets" ADD CONSTRAINT "webhook_targets_actor_id_users_id_fk" FOREIGN KEY ("actor_id") REFERENCES "public"."users"("id") ON DELETE set null ON UPDATE no action;