/**
 * Fixed copy for the `?error=` codes the /auth callbacks redirect with.
 * Provider-supplied error text is never shown: it is attacker-controllable.
 */
export const LOGIN_ERRORS: Record<string, string> = {
  oidc_state: "ActivSync could not verify the sign-in response. Sign in again.",
  oidc_mismatch: "That account is not linked to ActivSync. Sign in with the account you linked in Settings.",
  oidc_unavailable: "The single sign-on provider could not be reached. Use your password instead.",
  oidc_denied: "The single sign-on provider declined the sign-in.",
  link_password: "Enter your ActivSync password to link an account.",
};
