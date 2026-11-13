CREATE SEQUENCE public.control_resource_version_seq AS bigint NO CYCLE;
--> statement-breakpoint
ALTER TABLE public.upstreams ADD COLUMN row_version bigint NOT NULL DEFAULT nextval('public.control_resource_version_seq');
--> statement-breakpoint
ALTER TABLE public.nginx_servers ADD COLUMN row_version bigint NOT NULL DEFAULT nextval('public.control_resource_version_seq');
--> statement-breakpoint
ALTER TABLE public.nginx_streams ADD COLUMN row_version bigint NOT NULL DEFAULT nextval('public.control_resource_version_seq');
--> statement-breakpoint
-- Defaults backfill existing rows; the trigger also overrides explicit/old versions on reinsertion.
-- Sequence gaps (including an insert's default) are intentional and never reused on rollback.
CREATE FUNCTION public.assign_control_resource_version() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    NEW.row_version := nextval('public.control_resource_version_seq');
    RETURN NEW;
END;
$$;
--> statement-breakpoint
CREATE TRIGGER control_resource_version BEFORE INSERT OR UPDATE ON public.upstreams FOR EACH ROW EXECUTE FUNCTION public.assign_control_resource_version();
--> statement-breakpoint
CREATE TRIGGER control_resource_version BEFORE INSERT OR UPDATE ON public.nginx_servers FOR EACH ROW EXECUTE FUNCTION public.assign_control_resource_version();
--> statement-breakpoint
CREATE TRIGGER control_resource_version BEFORE INSERT OR UPDATE ON public.nginx_streams FOR EACH ROW EXECUTE FUNCTION public.assign_control_resource_version();
