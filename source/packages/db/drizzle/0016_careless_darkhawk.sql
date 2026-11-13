CREATE TABLE "mail_recipients" (
	"id" text PRIMARY KEY NOT NULL,
	"label" text,
	"address" text NOT NULL,
	"enabled" boolean DEFAULT true NOT NULL,
	"events" jsonb NOT NULL,
	"last_attempt_at" timestamp with time zone,
	"last_success_at" timestamp with time zone,
	"last_error" text,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	"actor_id" text
);
--> statement-breakpoint
ALTER TABLE "mail_recipients" ADD CONSTRAINT "mail_recipients_actor_id_users_id_fk" FOREIGN KEY ("actor_id") REFERENCES "public"."users"("id") ON DELETE set null ON UPDATE no action;--> statement-breakpoint
-- 업그레이드 안전장치: 이 표가 생기기 전에는 만기 안내가 "그 인증서의 ACME 계정
-- 주소"로만 나갔다. 빈 표로 두면 업그레이드하는 순간 그 메일이 조용히 끊긴다.
-- 이미 쓰이던 주소를 그대로 옮겨 담아, 화면을 열지 않아도 받던 사람이 계속 받는다.
-- 옮긴 뒤로는 평범한 행이라 운영자가 지우거나 고칠 수 있다.
--
-- staging/production 계정이 같은 주소를 쓰는 일이 흔해 DISTINCT 로 묶는다.
INSERT INTO "mail_recipients" ("id", "label", "address", "enabled", "events")
SELECT gen_random_uuid()::text, 'ACME 계정 연락처 (자동 등록)', email, true, '["cert.renewal_due"]'::jsonb
FROM (SELECT DISTINCT "email" FROM "acme_accounts" WHERE "email" IS NOT NULL AND "email" <> '') AS a;