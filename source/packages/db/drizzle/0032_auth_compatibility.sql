-- Existing rows retain the released pairing/issuer rules; only future rows opt into new policy.
ALTER TABLE public.access_tokens ADD COLUMN legacy_pairing boolean NOT NULL DEFAULT true;
--> statement-breakpoint
ALTER TABLE public.access_tokens ALTER COLUMN legacy_pairing SET DEFAULT false;
--> statement-breakpoint
ALTER TABLE public.gateway_tokens ADD COLUMN legacy_issuer boolean NOT NULL DEFAULT true;
--> statement-breakpoint
ALTER TABLE public.gateway_tokens ALTER COLUMN legacy_issuer SET DEFAULT false;
--> statement-breakpoint
ALTER TABLE public.gateway_tokens ADD COLUMN access_token_id text REFERENCES public.access_tokens(id);
--> statement-breakpoint
CREATE INDEX gateway_tokens_access_token_id_idx ON public.gateway_tokens(access_token_id) WHERE access_token_id IS NOT NULL;
