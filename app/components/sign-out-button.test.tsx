import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { toast } from "sonner";
import { afterEach, beforeEach, expect, test, vi } from "vitest";

import { ApiError } from "@/lib/api";
import { SignOutButton } from "./sign-out-button";

const { logout } = vi.hoisted(() => ({ logout: vi.fn() }));

vi.mock("@/lib/auth-api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/auth-api")>();
  return { ...actual, logout };
});

vi.mock("sonner", () => ({ toast: { error: vi.fn() } }));

function renderButton(props: Parameters<typeof SignOutButton>[0]) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <SignOutButton {...props} />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
});

afterEach(() => {
  vi.unstubAllGlobals();
});

test("signing out lands on the local sign-in form so single sign-on cannot re-enter", async () => {
  const assign = vi.fn();
  vi.stubGlobal("location", { ...window.location, assign });
  logout.mockResolvedValue(undefined);
  renderButton({ method: "password" });

  await userEvent.click(screen.getByRole("button", { name: "Sign out" }));

  await vi.waitFor(() => expect(assign).toHaveBeenCalledWith("/login?local=1"));
});

test("the text variant signs out the same way", async () => {
  const assign = vi.fn();
  vi.stubGlobal("location", { ...window.location, assign });
  logout.mockResolvedValue(undefined);
  renderButton({ method: "oidc", variant: "text" });

  await userEvent.click(screen.getByRole("button", { name: "Sign out" }));

  await vi.waitFor(() => expect(assign).toHaveBeenCalledWith("/login?local=1"));
});

test("a failed sign-out stays on the page and says why", async () => {
  const assign = vi.fn();
  vi.stubGlobal("location", { ...window.location, assign });
  logout.mockRejectedValue(new ApiError("Could not sign out.", 500));
  renderButton({ method: "password" });

  await userEvent.click(screen.getByRole("button", { name: "Sign out" }));

  await vi.waitFor(() => expect(toast.error).toHaveBeenCalledWith("Could not sign out."));
  expect(assign).not.toHaveBeenCalled();
});

test.each(["proxy", "disabled", null] as const)("%s sessions get no sign-out button", (method) => {
  renderButton({ method });

  expect(screen.queryByRole("button", { name: "Sign out" })).toBeNull();
});
