CREATE TABLE "app_settings" (
	"id" text PRIMARY KEY NOT NULL,
	"require_2fa_admin" boolean DEFAULT false NOT NULL,
	"require_2fa_user" boolean DEFAULT false NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	"actor_id" text
);
--> statement-breakpoint
ALTER TABLE "app_settings" ADD CONSTRAINT "app_settings_actor_id_users_id_fk" FOREIGN KEY ("actor_id") REFERENCES "public"."users"("id") ON DELETE no action ON UPDATE no action;