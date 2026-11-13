CREATE TABLE "applied_config" (
	"id" text PRIMARY KEY NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	"actor_id" text,
	"data" jsonb NOT NULL
);
--> statement-breakpoint
ALTER TABLE "applied_config" ADD CONSTRAINT "applied_config_actor_id_users_id_fk" FOREIGN KEY ("actor_id") REFERENCES "public"."users"("id") ON DELETE no action ON UPDATE no action;