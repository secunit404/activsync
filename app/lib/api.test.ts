import { afterEach, describe, expect, it, vi } from "vitest";

import {
  ApiError,
  getActivities,
  publishActivities,
  publishActivity,
} from "@/lib/api";

/**
 * Every component test mocks `@/lib/api`, so nothing else checks that the
 * client builds the URL, method and body the server actually expects. A typo
 * here is invisible until Playwright.
 */

function mockFetch(response: Partial<Response> & { json?: () => unknown }) {
  const fetchMock = vi.fn().mockResolvedValue({
    ok: true,
    status: 200,
    json: async () => ({}),
    ...response,
  } as Response);
  vi.stubGlobal("fetch", fetchMock);
  return fetchMock;
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("requestJson", () => {
  it("sends JSON headers and returns the parsed payload", async () => {
    const fetchMock = mockFetch({ json: async () => ({ message: "ok" }) });

    const result = await publishActivity(42);

    expect(result).toEqual({ message: "ok" });
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("/api/v1/activities/42/publish");
    expect(init.method).toBe("POST");
    expect(init.headers).toMatchObject({
      Accept: "application/json",
      "Content-Type": "application/json",
    });
  });

  it("serialises a request body", async () => {
    const fetchMock = mockFetch({});

    await publishActivities([1, 2]);

    const [, init] = fetchMock.mock.calls[0];
    expect(JSON.parse(init.body as string)).toEqual({ activityIds: [1, 2] });
  });

  it("puts query state in the URL rather than a body", async () => {
    const fetchMock = mockFetch({ json: async () => ({ activities: [] }) });

    await getActivities({ sort: "oldest", status: "held", page: 2, pageSize: 50 });

    const [url] = fetchMock.mock.calls[0];
    expect(url).toContain("sort=oldest");
    expect(url).toContain("status=held");
    expect(url).toContain("pageSize=50");
  });

  it("raises ApiError carrying the server's detail and status", async () => {
    mockFetch({
      ok: false,
      status: 409,
      json: async () => ({ detail: "Nothing to retry." }),
    });

    await expect(publishActivity(42)).rejects.toMatchObject({
      name: "ApiError",
      message: "Nothing to retry.",
      status: 409,
    });
  });

  it("falls back to a generic message when the error body has no detail", async () => {
    mockFetch({ ok: false, status: 500, json: async () => null });

    await expect(publishActivity(42)).rejects.toThrow(
      "ActivSync request failed (500)",
    );
  });

  it("returns undefined for 204 instead of parsing an empty body", async () => {
    mockFetch({
      status: 204,
      json: async () => {
        throw new Error("must not parse a 204 body");
      },
    });

    await expect(publishActivity(42)).resolves.toBeUndefined();
  });

  it("exposes ApiError as an Error subclass so catch blocks can narrow", () => {
    const error = new ApiError("nope", 400);
    expect(error).toBeInstanceOf(Error);
    expect(error.status).toBe(400);
  });
});
