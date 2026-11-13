/** Browser command state. Raw tokens only appear in the one-time issue response. */
export interface IssueTokenState {
  ok: boolean; token?: string; id?: string; name?: string;
  fieldErrors?: Partial<Record<"name" | "scopes" | "expiresAt", string>>;
  error?: string;
}
export interface RevokeTokenState { ok: boolean; error?: string }
