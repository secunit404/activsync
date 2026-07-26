import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { beforeEach, expect, test, vi } from "vitest";

import type { SettingsState } from "@/lib/api";
import { SetupView } from "./setup";

const {
  setupGarmin,
  completeGarminMfa,
  cancelGarminMfa,
  saveStravaCredentials,
  setupHevy,
  skipHevy,
  runInitialSync,
} = vi.hoisted(() => ({
  setupGarmin: vi.fn(),
  completeGarminMfa: vi.fn(),
  cancelGarminMfa: vi.fn(),
  saveStravaCredentials: vi.fn(),
  setupHevy: vi.fn(),
  skipHevy: vi.fn(),
  runInitialSync: vi.fn(),
}));

vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  return {
    ...actual,
    setupGarmin,
    completeGarminMfa,
    cancelGarminMfa,
    saveStravaCredentials,
    setupHevy,
    skipHevy,
    runInitialSync,
  };
});

const actionResult = { message: "ok", setupStep: null, mfaRequired: false };

beforeEach(() => {
  vi.clearAllMocks();
});

const baseState: SettingsState = {
  version: "0.1.0",
  update: {
    latest: null,
    available: false,
    repoUrl: "https://github.com/secunit404/activsync",
    releaseUrl: "https://github.com/secunit404/activsync/releases/latest",
  },
  development: true,
  setup: { complete: false, step: "garmin", mfaRequired: false },
  connections: {
    garmin: {
      connected: false,
      status: "Disconnected — sync paused",
      meta: "",
      email: "",
    },
    strava: { connected: false, status: "Disconnected — publishing paused", meta: "" },
    broken: ["garmin", "strava"],
  },
  credentials: {
    garminEmail: "",
    garminPasswordSaved: false,
    stravaClientId: "",
    stravaClientSecretSaved: false,
  },
  preferences: {
    displayTimezone: "Europe/Stockholm",
    garminPollIntervalMinutes: 20,
    stravaPollIntervalMinutes: 5,
    lookbackDays: 7,
  },
  timezones: ["Europe/Stockholm"],
  activityTypes: [],
  hevy: {
    connected: false,
    status: "",
    apiKeySaved: false,
    enabled: false,
    watchStrategy: "replace",
    matchMode: "automatic",
    descriptionTemplate: "{title}\n{exercises}",
    summaryOnStructured: true,
    graceMinutes: 120,
    pollIntervalMinutes: 10,
    identity: { manufacturer: null, product: null, serial: null },
    identityDisplay: "not yet detected",
    profileOverride: { weightKg: null, birthYear: null, vo2max: null, sex: null },
  profileBaseline: { weightKg: 80, birthYear: 1990, vo2max: 45, sex: "male" },
  profileFromGarmin: false,
  },
};

function renderSetup(state: SettingsState, stravaError?: string) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <SetupView state={state} stravaError={stravaError} />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

test("renders the Garmin connect form with the accepted history-window wording", () => {
  renderSetup(baseState);
  expect(
    screen.getByRole("heading", { name: "Connect your Garmin account" }),
  ).toBeInTheDocument();
  expect(screen.getByLabelText("Email")).toBeInTheDocument();
  expect(screen.getByLabelText("Password")).toBeInTheDocument();
  expect(
    screen.getByLabelText("Activity history window (days)"),
  ).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Connect Garmin" })).toBeEnabled();
});

test("submitting the Garmin form calls setupGarmin with the typed credentials", async () => {
  setupGarmin.mockResolvedValue(actionResult);
  renderSetup(baseState);
  await userEvent.type(screen.getByLabelText("Email"), "athlete@example.com");
  await userEvent.type(screen.getByLabelText("Password"), "hunter2");
  await userEvent.click(screen.getByRole("button", { name: "Connect Garmin" }));
  expect(setupGarmin).toHaveBeenCalledTimes(1);
  expect(setupGarmin.mock.calls[0][0]).toEqual(
    expect.objectContaining({ email: "athlete@example.com", password: "hunter2" }),
  );
});

test("the MFA overlay opens when the settings say Garmin needs a code, and focuses the first digit", async () => {
  renderSetup({ ...baseState, setup: { ...baseState.setup, mfaRequired: true } });
  const dialog = await screen.findByRole("dialog", { name: /verification code/i });
  expect(dialog).toBeVisible();
  await vi.waitFor(() =>
    expect(screen.getByRole("textbox", { name: /digit 1/i })).toHaveFocus(),
  );
});

test("completing all six digits enables Verify, which submits the code", async () => {
  completeGarminMfa.mockResolvedValue(actionResult);
  renderSetup({ ...baseState, setup: { ...baseState.setup, mfaRequired: true } });
  await screen.findByRole("dialog", { name: /verification code/i });
  const verifyButtons = screen.getAllByRole("button", { name: "Verify" });
  for (const button of verifyButtons) {
    expect(button).toBeDisabled();
  }
  await userEvent.type(screen.getByRole("textbox", { name: /digit 1/i }), "482913");
  for (const button of screen.getAllByRole("button", { name: "Verify" })) {
    expect(button).toBeEnabled();
  }
  await userEvent.click(screen.getAllByRole("button", { name: "Verify" })[0]);
  expect(completeGarminMfa).toHaveBeenCalledTimes(1);
  expect(completeGarminMfa.mock.calls[0][0]).toBe("482913");
});

test("cancelling MFA calls cancelGarminMfa", async () => {
  cancelGarminMfa.mockResolvedValue(actionResult);
  renderSetup({ ...baseState, setup: { ...baseState.setup, mfaRequired: true } });
  await screen.findByRole("dialog", { name: /verification code/i });
  await userEvent.click(screen.getAllByRole("button", { name: "Cancel" })[0]);
  expect(cancelGarminMfa).toHaveBeenCalledTimes(1);
});

test("the Garmin form is reachable underneath — mfaRequired: false never dead-ends the wizard", () => {
  // The overlay is layered on top of the step's own content rather than
  // replacing it (frames 2d + 3c together), so once mfaRequired flips back
  // to false — the state a real `cancelGarminMfa` success drives via query
  // invalidation — the underlying Garmin form is what's left, recoverable.
  renderSetup({ ...baseState, setup: { ...baseState.setup, mfaRequired: false } });
  expect(
    screen.getByRole("heading", { name: "Connect your Garmin account" }),
  ).toBeInTheDocument();
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});

test("dismissing the overlay (backdrop/Escape/close) also cancels the pending MFA session", async () => {
  cancelGarminMfa.mockResolvedValue(actionResult);
  renderSetup({ ...baseState, setup: { ...baseState.setup, mfaRequired: true } });
  await screen.findByRole("dialog", { name: /verification code/i });
  await userEvent.keyboard("{Escape}");
  expect(cancelGarminMfa).toHaveBeenCalled();
});

test("resending calls setupGarmin again with the same credentials", async () => {
  setupGarmin.mockResolvedValue(actionResult);
  renderSetup({
    ...baseState,
    setup: { ...baseState.setup, mfaRequired: true },
    credentials: { ...baseState.credentials, garminEmail: "athlete@example.com" },
  });
  await screen.findByRole("dialog", { name: /verification code/i });
  await userEvent.click(screen.getAllByRole("button", { name: "Resend code" })[0]);
  expect(setupGarmin).toHaveBeenCalledTimes(1);
  expect(setupGarmin.mock.calls[0][0]).toEqual(
    expect.objectContaining({ email: "athlete@example.com" }),
  );
});

test("renders the Strava step", () => {
  renderSetup({ ...baseState, setup: { ...baseState.setup, step: "strava" } });
  expect(
    screen.getByRole("heading", { name: "Authorize Strava" }),
  ).toBeInTheDocument();
  expect(screen.getByLabelText("Strava client ID")).toBeInTheDocument();
});

test("renders the Hevy step with a skip option", async () => {
  skipHevy.mockResolvedValue(actionResult);
  renderSetup({ ...baseState, setup: { ...baseState.setup, step: "hevy" } });
  await userEvent.click(screen.getByRole("button", { name: "Skip for now" }));
  expect(skipHevy).toHaveBeenCalled();
});

test("renders the sync step and triggers the initial sync", async () => {
  runInitialSync.mockResolvedValue(actionResult);
  renderSetup({ ...baseState, setup: { ...baseState.setup, step: "syncing" } });
  await userEvent.click(screen.getByRole("button", { name: "Start initial sync" }));
  expect(runInitialSync).toHaveBeenCalled();
});

test("renders the completion screen once setup is done", () => {
  renderSetup({ ...baseState, setup: { complete: true, step: null, mfaRequired: false } });
  expect(screen.getByRole("heading", { name: "You’re all set" })).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Go to ActivSync" })).toHaveAttribute(
    "href",
    "/",
  );
});

test("surfaces a Strava OAuth failure passed in from the callback redirect", () => {
  renderSetup(
    { ...baseState, setup: { ...baseState.setup, step: "strava" } },
    "Strava authorization was declined.",
  );
  expect(screen.getByText("Strava connection failed")).toBeInTheDocument();
  expect(screen.getByText("Strava authorization was declined.")).toBeInTheDocument();
});
