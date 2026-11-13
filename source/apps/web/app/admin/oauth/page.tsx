import { readPageApi } from "@/lib/api-page";
import type { ProviderView } from "./form-types";
import { ApiForm } from "@/components/api-form";
import { Button } from "@repo/ui";

import { ProviderListView } from "./provider-list";
export const runtime = "nodejs";
export const dynamic = "force-dynamic";
export default async function OAuthProvidersPage() {
  const providers = await readPageApi<ProviderView[]>("/api/ui/v1/admin/oauth");

  return (
    <ProviderListView
      providers={providers}
      toggleForm={(p) => (
        <ApiForm action={{ id: "admin.oauth.toggle", args: [] }}>
          <input type="hidden" name="id" value={p.id} />
          <Button size="sm" type="submit" variant="secondary">
            {p.enabled ? "사용 안 함" : "사용"}
          </Button>
        </ApiForm>
      )}
    />
  );
}
