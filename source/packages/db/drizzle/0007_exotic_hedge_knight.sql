CREATE TYPE "public"."cert_source" AS ENUM('acme', 'manual');--> statement-breakpoint
ALTER TABLE "certificates" ADD COLUMN "source" "cert_source" DEFAULT 'acme' NOT NULL;