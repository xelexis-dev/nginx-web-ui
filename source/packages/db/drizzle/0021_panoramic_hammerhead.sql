CREATE TYPE "public"."node_status" AS ENUM('ok', 'unreachable', 'unauthorized', 'version_mismatch', 'error');--> statement-breakpoint
CREATE TABLE "nodes" (
	"id" text PRIMARY KEY NOT NULL,
	"name" text NOT NULL,
	"base_url" text NOT NULL,
	"token_enc" text NOT NULL,
	"token_role" "role" NOT NULL,
	"allow_insecure_tls" boolean DEFAULT false NOT NULL,
	"instance_id" text NOT NULL,
	"created_by_user_id" text,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"last_seen_at" timestamp with time zone,
	"last_status" "node_status",
	"last_version" text,
	"last_error" text,
	CONSTRAINT "nodes_instance_id_unique" UNIQUE("instance_id")
);
--> statement-breakpoint
ALTER TABLE "nodes" ADD CONSTRAINT "nodes_created_by_user_id_users_id_fk" FOREIGN KEY ("created_by_user_id") REFERENCES "public"."users"("id") ON DELETE set null ON UPDATE no action;