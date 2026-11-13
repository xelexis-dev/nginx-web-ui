CREATE TABLE "domain_metric_hours" (
	"bucket" timestamp with time zone NOT NULL,
	"host_id" integer NOT NULL,
	"requests" bigint NOT NULL,
	"bytes_sent" bigint NOT NULL,
	"s2xx" bigint NOT NULL,
	"s3xx" bigint NOT NULL,
	"s4xx" bigint NOT NULL,
	"s5xx" bigint NOT NULL,
	CONSTRAINT "domain_metric_hours_bucket_host_id_pk" PRIMARY KEY("bucket","host_id")
);
--> statement-breakpoint
CREATE TABLE "domain_metric_minutes" (
	"bucket" timestamp with time zone NOT NULL,
	"host_id" integer NOT NULL,
	"requests" integer NOT NULL,
	"bytes_sent" bigint NOT NULL,
	"s2xx" integer NOT NULL,
	"s3xx" integer NOT NULL,
	"s4xx" integer NOT NULL,
	"s5xx" integer NOT NULL,
	CONSTRAINT "domain_metric_minutes_bucket_host_id_pk" PRIMARY KEY("bucket","host_id")
);
--> statement-breakpoint
CREATE TABLE "metric_hosts" (
	"id" integer PRIMARY KEY GENERATED ALWAYS AS IDENTITY (sequence name "metric_hosts_id_seq" INCREMENT BY 1 MINVALUE 1 MAXVALUE 2147483647 START WITH 1 CACHE 1),
	"host" text NOT NULL,
	"first_seen_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "metric_hosts_host_unique" UNIQUE("host")
);
--> statement-breakpoint
CREATE TABLE "metric_parse_state" (
	"id" text PRIMARY KEY NOT NULL,
	"offset" bigint DEFAULT 0 NOT NULL,
	"size" bigint DEFAULT 0 NOT NULL,
	"ino" text,
	"head_fingerprint" text,
	"last_bucket" timestamp with time zone,
	"unparsed_count" bigint DEFAULT 0 NOT NULL,
	"last_rollup_at" timestamp with time zone,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "nginx_metric_samples" (
	"collected_at" timestamp with time zone PRIMARY KEY NOT NULL,
	"active" integer NOT NULL,
	"reading" integer NOT NULL,
	"writing" integer NOT NULL,
	"waiting" integer NOT NULL,
	"accepts" bigint NOT NULL,
	"handled" bigint NOT NULL,
	"requests" bigint NOT NULL
);
--> statement-breakpoint
ALTER TABLE "app_settings" ADD COLUMN "metrics_retention_days" integer;--> statement-breakpoint
ALTER TABLE "app_settings" ADD COLUMN "metrics_excluded_hosts" text;--> statement-breakpoint
ALTER TABLE "domain_metric_hours" ADD CONSTRAINT "domain_metric_hours_host_id_metric_hosts_id_fk" FOREIGN KEY ("host_id") REFERENCES "public"."metric_hosts"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "domain_metric_minutes" ADD CONSTRAINT "domain_metric_minutes_host_id_metric_hosts_id_fk" FOREIGN KEY ("host_id") REFERENCES "public"."metric_hosts"("id") ON DELETE cascade ON UPDATE no action;