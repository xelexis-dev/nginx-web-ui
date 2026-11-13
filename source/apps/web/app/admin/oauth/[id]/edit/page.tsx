import layout from "@/components/page-layout.module.css";
import { buttonClassName, PageHeader } from "@repo/ui";
import Link from "next/link";
import { readPageApi } from "@/lib/api-page";
import type { ProviderView } from "../../form-types";
import { ProviderForm } from "../../provider-form";
import styles from "../../oauth.module.css";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export default async function EditOAuthProviderPage({ params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  const provider = await readPageApi<ProviderView & { callbackUrl: string }>(`/api/ui/v1/admin/oauth/${encodeURIComponent(id)}`);

  return (
    <section className={layout.page}>
      <PageHeader
        title={<>공급자 수정</>}
        description={<>{provider.name}</>}
        actions={
          <>
            {" "}
            <div className={styles.headerActions}>
              <Link href={`/admin/oauth/${provider.id}`} className={buttonClassName({ variant: "secondary" })}>
                상세로
              </Link>
            </div>{" "}
          </>
        }
      />

      <ProviderForm
        mode="edit"
        defaults={{
          id: provider.id,
          name: provider.name,
          issuer: provider.issuer,
          clientId: provider.clientId,
          scope: provider.scope,
          enabled: provider.enabled,
          sortOrder: provider.sortOrder,
        }}
        callbackUrl={provider.callbackUrl}
      />
    </section>
  );
}
