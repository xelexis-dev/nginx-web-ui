export interface ProviderFormState {
  ok: boolean;
  error?: string;
  fieldErrors?: Record<string, string>;
}
export type { ProviderView } from "@repo/contracts";
