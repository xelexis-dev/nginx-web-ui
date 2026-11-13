CREATE TYPE "public"."challenge_type" AS ENUM('http-01', 'dns-01');--> statement-breakpoint
ALTER TYPE "public"."cert_status" ADD VALUE 'awaiting-dns';--> statement-breakpoint
ALTER TABLE "certificates" ADD COLUMN "challenge_type" "challenge_type" DEFAULT 'http-01' NOT NULL;--> statement-breakpoint
ALTER TABLE "certificates" ADD COLUMN "dns_order" jsonb;