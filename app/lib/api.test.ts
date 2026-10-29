import { afterEach, describe, expect, it, vi } from "vitest";

import {
  ApiError,
  getActivities,
  getAppState,
  loginPath,
  cancelPublish,
  getPublishBatch,
  schedulePublishActivities,
  schedulePublishActivity,
} from "@/lib/api";
import { login } from "@/lib/auth-api";

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

    const result = await schedulePublishActivity(42);

    expect(result).toEqual({ message: "ok" });
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("/api/v1/activities/42/publish/schedule");
    expect(init.method).toBe("POST");
    expect(init.headers).toMatchObject({
      Accept: "application/json",
      "Content-Type": "application/json",
    });
  });

  it("serialises a request body", async () => {
    const fetchMock = mockFetch({});

    await schedulePublishActivities([1, 2]);

    const [, init] = fetchMock.mock.calls[0];
    expect(JSON.parse(init.body as string)).toEqual({ activityIds: [1, 2] });
  });

  it("cancels one publish by id and several in one body", async () => {
    const fetchMock = mockFetch({});

    await cancelPublish([42]);
    await cancelPublish([1, 2]);

    const [[oneUrl, oneInit], [manyUrl, manyInit]] = fetchMock.mock.calls;
    expect(oneUrl).toBe("/api/v1/activities/42/publish/cancel");
    expect(oneInit.method).toBe("POST");
    expect(manyUrl).toBe("/api/v1/activities/publish/cancel");
    expect(JSON.parse(manyInit.body as string)).toEqual({ activityIds: [1, 2] });
  });

  it("reads a publish batch by id", async () => {
    const fetchMock = mockFetch({});

    await getPublishBatch("abc");

    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("/api/v1/activities/publish/batches/abc");
    expect(init.method).toBe("GET");
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

    await expect(schedulePublishActivity(42)).rejects.toMatchObject({
      name: "ApiError",
      message: "Nothing to retry.",
      status: 409,
    });
  });

  it("falls back to a generic message when the error body has no detail", async () => {
    mockFetch({ ok: false, status: 500, json: async () => null });

    await expect(schedulePublishActivity(42)).rejects.toThrow(
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

    await expect(schedulePublishActivity(42)).resolves.toBeUndefined();
  });

  it("exposes ApiError as an Error subclass so catch blocks can narrow", () => {
    const error = new ApiError("nope", 400);
    expect(error).toBeInstanceOf(Error);
    expect(error.status).toBe(400);
  });

  it("marks every request with the CSRF header the server requires", async () => {
    const fetchMock = mockFetch({});

    await schedulePublishActivity(42);

    const [, init] = fetchMock.mock.calls[0];
    expect(init.headers).toMatchObject({ "X-ActivSync-Request": "1" });
  });

  it("surfaces Retry-After on a 429", async () => {
    mockFetch({
      ok: false,
      status: 429,
      headers: new Headers({ "Retry-After": "30" }),
      json: async () => ({ detail: "Too many attempts. Try again in 30 seconds." }),
    });

    await expect(login("guess")).rejects.toMatchObject({
      status: 429,
      retryAfterSeconds: 30,
    });
  });
});

describe("signed-out handling", () => {
  function stubLocation(pathname: string, search = "") {
    const assign = vi.fn();
    vi.stubGlobal("location", { ...window.location, pathname, search, assign });
    return assign;
  }

  it("sends the browser to login with the current page as next on a 401", async () => {
    const assign = stubLocation("/hevy/mappings", "?q=row");
    mockFetch({ ok: false, status: 401, json: async () => ({ detail: "Not signed in" }) });

    await expect(getAppState()).rejects.toMatchObject({ status: 401 });

    expect(assign).toHaveBeenCalledWith("/login?next=%2Fhevy%2Fmappings%3Fq%3Drow");
  });

  it("leaves auth endpoints' own 401s to the caller", async () => {
    const assign = stubLocation("/login");
    mockFetch({ ok: false, status: 401, json: async () => ({ detail: "Wrong password." }) });

    await expect(login("nope")).rejects.toMatchObject({ message: "Wrong password." });

    expect(assign).not.toHaveBeenCalled();
  });

  it("builds a bare /login for the root path", () => {
    expect(loginPath("/")).toBe("/login");
    expect(loginPath("/settings")).toBe("/login?next=%2Fsettings");
  });
});
