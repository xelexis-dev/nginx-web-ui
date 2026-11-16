import layout from "@/components/page-layout.module.css";
import { PageHeader } from "@repo/ui";
import { readAdminSession } from "@/lib/admin-api";
import { ProviderForm } from "../provider-form";
export const runtime = "nodejs";
export const dynamic = "force-dynamic";
export default async function NewOAuthProviderPage() {
  await readAdminSession();
  return (
    <section className={layout.page}>
      <PageHeader title={<>공급자 추가</>} />
      <ProviderForm mode="create" />
    </section>
  );
}
