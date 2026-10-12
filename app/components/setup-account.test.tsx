import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, test, vi } from "vitest";

import { ApiError } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import { AccountStep } from "./setup-account";

const { claimOwner } = vi.hoisted(() => ({ claimOwner: vi.fn() }));

vi.mock("@/lib/auth-api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/auth-api")>();
  return { ...actual, claimOwner };
});

const PASSWORD = "correct horse battery staple";

function renderStep() {
  const client = new QueryClient({ defaultOptions: { mutations: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <AccountStep />
    </QueryClientProvider>,
  );
  return client;
}

beforeEach(() => {
  claimOwner.mockReset();
});

test("explains where the setup code comes from", () => {
  renderStep();
  expect(screen.getByText(/docker compose logs activsync/)).toBeVisible();
});

test("claims with the code and password and stores the signed-in status", async () => {
  const signedIn = {
    authenticated: true,
    method: "password",
    user: null,
    ownerClaimed: true,
    authDisabled: false,
    login: { oidc: null, strava: false },
  };
  claimOwner.mockResolvedValue(signedIn);
  const client = renderStep();

  await userEvent.type(screen.getByLabelText("Setup code"), "ABCD-EFGH-JKMN-PQRS");
  await userEvent.type(screen.getByLabelText("Password"), PASSWORD);
  await userEvent.type(screen.getByLabelText("Repeat password"), PASSWORD);
  await userEvent.click(screen.getByRole("button", { name: "Claim ActivSync" }));

  await vi.waitFor(() =>
    expect(claimOwner).toHaveBeenCalledWith(
      { setupCode: "ABCD-EFGH-JKMN-PQRS", password: PASSWORD },
      expect.anything(),
    ),
  );
  await vi.waitFor(() => expect(client.getQueryData(queryKeys.authStatus)).toEqual(signedIn));
});

test("blocks a short or mismatched password before asking the server", async () => {
  renderStep();

  await userEvent.type(screen.getByLabelText("Setup code"), "ABCD-EFGH-JKMN-PQRS");
  await userEvent.type(screen.getByLabelText("Password"), "short");
  expect(screen.getByRole("button", { name: "Claim ActivSync" })).toBeDisabled();

  await userEvent.clear(screen.getByLabelText("Password"));
  await userEvent.type(screen.getByLabelText("Password"), PASSWORD);
  await userEvent.type(screen.getByLabelText("Repeat password"), PASSWORD + "x");
  expect(screen.getByText("The passwords don’t match.")).toBeVisible();
  expect(screen.getByRole("button", { name: "Claim ActivSync" })).toBeDisabled();
  expect(claimOwner).not.toHaveBeenCalled();
});

test("shows a wrong setup code", async () => {
  claimOwner.mockRejectedValue(new ApiError("That setup code is not right.", 403));
  renderStep();

  await userEvent.type(screen.getByLabelText("Setup code"), "WRNG-CODE-WRNG-CODE");
  await userEvent.type(screen.getByLabelText("Password"), PASSWORD);
  await userEvent.type(screen.getByLabelText("Repeat password"), PASSWORD);
  await userEvent.click(screen.getByRole("button", { name: "Claim ActivSync" }));

  expect(await screen.findByRole("alert")).toHaveTextContent("That setup code is not right.");
});

test("both password fields toggle visibility on their own", async () => {
  renderStep();

  const password = screen.getByLabelText("Password");
  const repeat = screen.getByLabelText("Repeat password");
  expect(password).toHaveAttribute("autocomplete", "new-password");
  expect(repeat).toHaveAttribute("autocomplete", "new-password");

  const toggles = screen.getAllByRole("button", { name: "Show password" });
  expect(toggles).toHaveLength(2);
  await userEvent.click(toggles[1]);

  expect(password).toHaveAttribute("type", "password");
  expect(repeat).toHaveAttribute("type", "text");
  expect(screen.getByRole("button", { name: "Hide password" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
});
