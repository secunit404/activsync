import { act, renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { PropsWithChildren } from "react";
import { beforeEach, expect, test, vi } from "vitest";

import { useActivityActions } from "./use-activity-actions";

const { excludeActivity } = vi.hoisted(() => ({
  excludeActivity: vi.fn(),
}));

vi.mock("@/lib/api", () => ({
  editActivity: vi.fn(),
  excludeActivity,
  publishActivities: vi.fn(),
  publishActivity: vi.fn(),
  restoreActivity: vi.fn(),
}));

function renderActions() {
  const queryClient = new QueryClient();
  const wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
  return renderHook(() => useActivityActions(), { wrapper });
}

beforeEach(() => {
  excludeActivity.mockReset();
});

// There is no bulk-exclude endpoint, so "exclude-many" fans out to one
// excludeActivity call per id and folds the settled results into a single
// ActivityActionResult (see the dedicated excludeMany() helper).
test("exclude-many succeeds when every call succeeds", async () => {
  excludeActivity.mockResolvedValue({
    message: "Excluded",
    severity: "success",
    publishedCount: 0,
    failedCount: 0,
    blockedCount: 0,
  });
  const { result } = renderActions();

  act(() => {
    result.current.mutate({ type: "exclude-many", activityIds: [1, 2] });
  });

  await waitFor(() => expect(result.current.isSuccess).toBe(true));
  expect(excludeActivity).toHaveBeenCalledWith(1);
  expect(excludeActivity).toHaveBeenCalledWith(2);
  expect(result.current.data?.severity).toBe("success");
  expect(result.current.data?.message).toBe("Excluded 2 activities");
});

test("exclude-many resolves with a warning on partial failure", async () => {
  excludeActivity.mockImplementation((id: number) =>
    id === 1 ? Promise.resolve({ message: "ok", severity: "success" as const, publishedCount: 0, failedCount: 0, blockedCount: 0 }) : Promise.reject(new Error("409")),
  );
  const { result } = renderActions();

  act(() => {
    result.current.mutate({ type: "exclude-many", activityIds: [1, 2] });
  });

  await waitFor(() => expect(result.current.isSuccess).toBe(true));
  expect(result.current.data?.severity).toBe("warning");
  expect(result.current.data?.message).toBe("Excluded 1; 1 failed and stayed as-is");
});

test("exclude-many rejects when every call fails", async () => {
  excludeActivity.mockRejectedValue(new Error("409"));
  const { result } = renderActions();

  act(() => {
    result.current.mutate({ type: "exclude-many", activityIds: [1, 2] });
  });

  await waitFor(() => expect(result.current.isError).toBe(true));
  expect(result.current.error?.message).toBe("Could not exclude 2 activities");
});
