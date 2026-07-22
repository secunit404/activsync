import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { beforeEach, expect, test, vi } from "vitest";

import type { SettingsState } from "@/lib/api";
import { ConnectionsSettings } from "./settings-connections";

const { saveStravaCredentials, saveHevyCredentials } = vi.hoisted(() => ({
  saveStravaCredentials: vi.fn(),
  saveHevyCredentials: vi.fn(),
}));

vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  return { ...actual, saveStravaCredentials, saveHevyCredentials };
});

const baseState: SettingsState = {
  version: "0.1.0",
  update: {
    latest: null,
    available: false,
    repoUrl: "https://github.com/secunit404/activsync",
    releaseUrl: "https://github.com/secunit404/activsync/releases/latest",
  },
  development: false,
  setup: { complete: true, step: null, mfaRequired: false },
  connections: {
    garmin: {
      connected: true,
      status: "Connected",
      meta: "last synced 2 min ago",
      email: "athlete@example.com",
    },
    strava: { connected: true, status: "Connected", meta: "" },
    broken: [],
  },
  credentials: {
    garminEmail: "athlete@example.com",
    garminPasswordSaved: true,
    stravaClientId: "1234",
    stravaClientSecretSaved: true,
  },
  preferences: {
    displayTimezone: "Europe/Stockholm",
    garminPollIntervalMinutes: 20,
    stravaPollIntervalMinutes: 5,
    lookbackDays: 7,
    hevy2garminMarker: "— synced by hevy2garmin",
    hevy2garminMarkerEnabled: false,
  },
  timezones: ["Europe/Stockholm"],
  activityTypes: [],
  hevy: {
    connected: true,
    status: "Connected",
    apiKeySaved: true,
    enabled: true,
    watchStrategy: "replace",
    graceMinutes: 120,
    pollIntervalMinutes: 10,
    identity: { manufacturer: null, product: null, serial: null },
    identityDisplay: "not yet detected",
    profileOverride: { weightKg: null, birthYear: null, vo2max: null, sex: null },
  },
};

beforeEach(() => {
  vi.clearAllMocks();
});

function renderConnections(state: SettingsState = baseState) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  render(
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <ConnectionsSettings state={state} />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

/** Opens the dialog for one connection row, scoped by that row's testid so
 *  the three identical "Manage" triggers stay unambiguous. */
async function openDialog(row: string) {
  const status = screen.getByTestId(`connection-status-${row}`);
  await userEvent.click(within(status).getByRole("button"));
  return screen.getByRole("dialog");
}

// A connected Hevy dialog used to contain nothing but a Disconnect button,
// so there was no way to rotate a key without disconnecting first.
test("the connected Hevy dialog can replace the key, not just disconnect", async () => {
  renderConnections();
  const dialog = await openDialog("hevy");

  expect(within(dialog).getByText(/API key saved/)).toBeVisible();
  expect(within(dialog).getByLabelText("Replace API key")).toBeVisible();
  expect(within(dialog).getByRole("button", { name: "Save new key" })).toBeDisabled();
  expect(within(dialog).getByRole("button", { name: "Disconnect" })).toBeEnabled();
});

test("Save new key enables once a key is typed", async () => {
  renderConnections();
  const dialog = await openDialog("hevy");

  await userEvent.type(within(dialog).getByLabelText("Replace API key"), "new-key");
  expect(within(dialog).getByRole("button", { name: "Save new key" })).toBeEnabled();
});

test("a disconnected Hevy dialog asks for a key instead", async () => {
  renderConnections({ ...baseState, hevy: { ...baseState.hevy, connected: false } });
  const dialog = await openDialog("hevy");

  expect(within(dialog).getByLabelText("Hevy API key")).toBeVisible();
  expect(within(dialog).queryByRole("button", { name: "Disconnect" })).toBeNull();
});
