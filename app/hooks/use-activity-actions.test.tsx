import { act, renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { PropsWithChildren } from "react";
import { beforeEach, expect, test, vi } from "vitest";

import { useActivityActions, type ExcludeTarget } from "./use-activity-actions";

const { excludeActivity, restoreActivity, toast } = vi.hoisted(() => {
  const toast = Object.assign(vi.fn(), {
    success: vi.fn(),
    warning: vi.fn(),
    error: vi.fn(),
  });
  return { excludeActivity: vi.fn(), restoreActivity: vi.fn(), toast };
});

vi.mock("sonner", () => ({ toast }));

vi.mock("@/lib/api", () => ({
  editActivity: vi.fn(),
  excludeActivity,
  publishActivities: vi.fn(),
  publishActivity: vi.fn(),
  restoreActivity,
}));

function renderActions() {
  const queryClient = new QueryClient();
  const wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
  return renderHook(() => useActivityActions(), { wrapper });
}

beforeEach(() => {
  vi.clearAllMocks();
  excludeActivity.mockReset();
  restoreActivity.mockReset();
});

const ok = {
  message: "ok",
  severity: "success" as const,
  publishedCount: 0,
  failedCount: 0,
  blockedCount: 0,
};

const pending: ExcludeTarget = { garminActivityId: 1, publishStatus: "pending", holdReason: null };
const backlog: ExcludeTarget = { garminActivityId: 2, publishStatus: "held", holdReason: "backlog" };
const missing: ExcludeTarget = { garminActivityId: 3, publishStatus: "missing", holdReason: null };

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
    result.current.mutate({ type: "exclude-many", activities: [pending, backlog] });
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
    result.current.mutate({ type: "exclude-many", activities: [pending, backlog] });
  });

  await waitFor(() => expect(result.current.isSuccess).toBe(true));
  expect(result.current.data?.severity).toBe("warning");
  expect(result.current.data?.message).toBe("Excluded 1; 1 failed and stayed as-is");
});

test("exclude-many rejects when every call fails", async () => {
  excludeActivity.mockRejectedValue(new Error("409"));
  const { result } = renderActions();

  act(() => {
    result.current.mutate({ type: "exclude-many", activities: [pending, backlog] });
  });

  await waitFor(() => expect(result.current.isError).toBe(true));
  expect(result.current.error?.message).toBe("Could not exclude 2 activities");
});

function undoFromToast() {
  const options = toast.mock.calls.at(-1)?.[1] as { action: { label: string; onClick: () => void } };
  expect(options.action.label).toBe("Undo");
  options.action.onClick();
}

test("undoing a bulk exclude puts each row back where it was, not where auto-sync says", async () => {
  excludeActivity.mockResolvedValue(ok);
  restoreActivity.mockResolvedValue(ok);
  const { result } = renderActions();

  act(() => {
    result.current.mutate({ type: "exclude-many", activities: [pending, backlog, missing] });
  });
  await waitFor(() => expect(result.current.isSuccess).toBe(true));
  expect(toast).toHaveBeenCalledWith("Excluded 3 activities", expect.anything());

  undoFromToast();

  await waitFor(() => expect(restoreActivity).toHaveBeenCalledTimes(3));
  expect(restoreActivity).toHaveBeenCalledWith(1, { status: "pending", holdReason: null });
  expect(restoreActivity).toHaveBeenCalledWith(2, { status: "held", holdReason: "backlog" });
  expect(restoreActivity).toHaveBeenCalledWith(3, { status: "missing", holdReason: null });
  await waitFor(() =>
    expect(toast).toHaveBeenCalledWith("Restored 3 activities", {
      description: "Back where they were.",
    }),
  );
});

test("undo after a partial exclude only restores the rows that were excluded", async () => {
  excludeActivity.mockImplementation((id: number) =>
    id === 1 ? Promise.resolve(ok) : Promise.reject(new Error("409")),
  );
  restoreActivity.mockResolvedValue(ok);
  const { result } = renderActions();

  act(() => {
    result.current.mutate({ type: "exclude-many", activities: [pending, backlog] });
  });
  await waitFor(() => expect(result.current.isSuccess).toBe(true));
  const options = toast.warning.mock.calls.at(-1)?.[1] as { action: { onClick: () => void } };
  options.action.onClick();

  await waitFor(() => expect(restoreActivity).toHaveBeenCalledTimes(1));
  expect(restoreActivity).toHaveBeenCalledWith(1, { status: "pending", holdReason: null });
});

test("a single exclude offers the same Undo", async () => {
  excludeActivity.mockResolvedValue({ ...ok, message: "Excluded Morning run" });
  restoreActivity.mockResolvedValue(ok);
  const { result } = renderActions();

  act(() => {
    result.current.mutate({ type: "exclude", activity: backlog });
  });
  await waitFor(() => expect(result.current.isSuccess).toBe(true));
  expect(toast).toHaveBeenCalledWith("Excluded Morning run", expect.anything());

  undoFromToast();

  await waitFor(() =>
    expect(restoreActivity).toHaveBeenCalledWith(2, { status: "held", holdReason: "backlog" }),
  );
});

test("an undo that fails says so", async () => {
  excludeActivity.mockResolvedValue(ok);
  restoreActivity.mockRejectedValue(new Error("409"));
  const { result } = renderActions();

  act(() => {
    result.current.mutate({ type: "exclude", activity: pending });
  });
  await waitFor(() => expect(result.current.isSuccess).toBe(true));
  undoFromToast();

  await waitFor(() =>
    expect(toast.error).toHaveBeenCalledWith("Could not undo the exclude", expect.anything()),
  );
});
