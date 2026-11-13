CREATE TYPE "public"."cert_status" AS ENUM('issuing', 'issued', 'failed');--> statement-breakpoint
CREATE TABLE "acme_accounts" (
	"id" text PRIMARY KEY NOT NULL,
	"directory_url" text NOT NULL,
	"email" text NOT NULL,
	"account_key_pem" text NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "acme_accounts_directory_url_unique" UNIQUE("directory_url")
);
--> statement-breakpoint
CREATE TABLE "acme_challenges" (
	"token" text PRIMARY KEY NOT NULL,
	"key_authorization" text NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "certificates" (
	"id" text PRIMARY KEY NOT NULL,
	"domains" jsonb NOT NULL,
	"status" "cert_status" DEFAULT 'issuing' NOT NULL,
	"staging" boolean DEFAULT false NOT NULL,
	"cert_pem" text,
	"key_pem" text,
	"issued_at" timestamp with time zone,
	"expires_at" timestamp with time zone,
	"last_error" text,
	"last_attempt_at" timestamp with time zone,
	"failure_count" integer DEFAULT 0 NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"actor_id" text
);
--> statement-breakpoint
ALTER TABLE "nginx_servers" ADD COLUMN "certificate_id" text;--> statement-breakpoint
ALTER TABLE "certificates" ADD CONSTRAINT "certificates_actor_id_users_id_fk" FOREIGN KEY ("actor_id") REFERENCES "public"."users"("id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "nginx_servers" ADD CONSTRAINT "nginx_servers_certificate_id_certificates_id_fk" FOREIGN KEY ("certificate_id") REFERENCES "public"."certificates"("id") ON DELETE no action ON UPDATE no action;