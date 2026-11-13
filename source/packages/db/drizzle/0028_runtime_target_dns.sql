ALTER TABLE "upstreams" ADD COLUMN "resolve_at_runtime" boolean DEFAULT false NOT NULL;
--> statement-breakpoint
ALTER TABLE "nginx_streams" ADD COLUMN "resolve_at_runtime" boolean DEFAULT false NOT NULL;
