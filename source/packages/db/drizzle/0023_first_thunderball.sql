CREATE TYPE "public"."token_kind" AS ENUM('user', 'server');--> statement-breakpoint
CREATE TABLE "access_tokens" (
	"id" text PRIMARY KEY NOT NULL,
	"kind" "token_kind" NOT NULL,
	"owner_user_id" text,
	"name" text NOT NULL,
	"token_hash" text NOT NULL,
	"token_suffix" text NOT NULL,
	"scopes" jsonb NOT NULL,
	"expires_at" timestamp with time zone,
	"created_by_user_id" text,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"last_used_at" timestamp with time zone,
	"last_used_ip" text,
	"revoked_at" timestamp with time zone,
	CONSTRAINT "access_tokens_token_hash_unique" UNIQUE("token_hash"),
	CONSTRAINT "access_tokens_owner_matches_kind" CHECK (("access_tokens"."kind" = 'user' AND "access_tokens"."owner_user_id" IS NOT NULL) OR ("access_tokens"."kind" = 'server' AND "access_tokens"."owner_user_id" IS NULL)),
	CONSTRAINT "access_tokens_scopes_is_array" CHECK (jsonb_typeof("access_tokens"."scopes") = 'array'),
	CONSTRAINT "access_tokens_suffix_len" CHECK (char_length("access_tokens"."token_suffix") = 4)
);
--> statement-breakpoint
ALTER TABLE "access_tokens" ADD CONSTRAINT "access_tokens_owner_user_id_users_id_fk" FOREIGN KEY ("owner_user_id") REFERENCES "public"."users"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "access_tokens" ADD CONSTRAINT "access_tokens_created_by_user_id_users_id_fk" FOREIGN KEY ("created_by_user_id") REFERENCES "public"."users"("id") ON DELETE set null ON UPDATE no action;