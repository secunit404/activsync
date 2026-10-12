/**
 * Fixed copy for the `?error=` codes the /auth callbacks redirect with.
 * Provider-supplied error text is never shown: it is attacker-controllable.
 */
export const LOGIN_ERRORS: Record<string, string> = {
  oidc_state: "That sign-in response could not be verified. Start again.",
  oidc_mismatch: "That account is not linked to ActivSync. Sign in with the account you linked in Settings.",
  oidc_unavailable: "The single sign-on provider could not be reached. Use your password instead.",
  oidc_denied: "The single sign-on provider declined the sign-in.",
  strava_state: "That Strava response could not be verified. Start again.",
  strava_mismatch: "That Strava account is not the one connected to ActivSync.",
  strava_unavailable: "Sign in with Strava is not available right now. Use your password instead.",
  strava_denied: "Strava sign-in was declined.",
};
