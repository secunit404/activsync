import { expect, test } from "vitest";

import { LOGIN_ERRORS } from "@/lib/auth-errors";

test("every code the auth redirects use has copy", () => {
  for (const code of [
    "oidc_state",
    "oidc_mismatch",
    "oidc_unavailable",
    "oidc_denied",
    "link_password",
  ]) {
    expect(LOGIN_ERRORS[code]).toBeTruthy();
  }
});
