import { expect, type Page } from "@playwright/test";

export const BASE_URL = "http://127.0.0.1:8384";
export const OWNER_STATE = "e2e/.auth/owner.json";
export const SIGNED_OUT = { cookies: [], origins: [] };
// dev_seed.DEV_PASSWORD: the mock seed's owner, created only under MOCK_MODE.
export const DEV_PASSWORD = "activsync-dev-password";
export const CSRF_HEADERS = { "X-ActivSync-Request": "1", Origin: BASE_URL };

export async function signIn(page: Page, next = "/") {
  await page.goto(next === "/" ? "/login" : `/login?next=${encodeURIComponent(next)}`);
  await page.getByLabel("Password", { exact: true }).fill(DEV_PASSWORD);
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page).not.toHaveURL(/\/login/);
}
