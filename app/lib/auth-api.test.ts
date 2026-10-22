import { afterEach, expect, test, vi } from "vitest";

import { ApiError } from "@/lib/api";
import { OIDC_LOGO_MAX_BYTES, uploadOidcLogo } from "@/lib/auth-api";

afterEach(() => {
  vi.unstubAllGlobals();
});

test("a logo over 1 MB is refused before it is sent", async () => {
  const fetchMock = vi.fn();
  vi.stubGlobal("fetch", fetchMock);
  const file = new File([new Uint8Array(OIDC_LOGO_MAX_BYTES + 1)], "big.png", { type: "image/png" });

  await expect(uploadOidcLogo(file)).rejects.toEqual(
    new ApiError("That image is too large. Use one under 1 MB.", 413),
  );
  expect(fetchMock).not.toHaveBeenCalled();
});

test("a logo is sent as the raw file with its own type", async () => {
  const fetchMock = vi.fn().mockResolvedValue(
    new Response(JSON.stringify({ message: "Logo saved." }), { status: 200 }),
  );
  vi.stubGlobal("fetch", fetchMock);
  const file = new File([new Uint8Array([1, 2, 3])], "logo.webp", { type: "image/webp" });

  await expect(uploadOidcLogo(file)).resolves.toEqual({ message: "Logo saved." });

  const [url, init] = fetchMock.mock.calls[0];
  expect(url).toBe("/api/v1/auth/oidc/logo");
  expect(init.method).toBe("PUT");
  expect(init.body).toBe(file);
  expect(init.headers["Content-Type"]).toBe("image/webp");
  expect(init.headers["X-ActivSync-Request"]).toBe("1");
});
