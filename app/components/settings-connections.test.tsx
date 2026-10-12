import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { beforeEach, expect, test, vi } from "vitest";

import type { SettingsState } from "@/lib/api";
import { ConnectionsSettings } from "./settings-connections";

const { saveStravaCredentials, saveHevyCredentials, startStravaConnect } = vi.hoisted(
  () => ({
    saveStravaCredentials: vi.fn(),
    saveHevyCredentials: vi.fn(),
    startStravaConnect: vi.fn(),
  }),
);

vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  return { ...actual, saveStravaCredentials, saveHevyCredentials, startStravaConnect };
});

const STRAVA_AUTHORIZE =
  "https://www.strava.com/oauth/authorize?client_id=1234&state=issued";

const baseState: SettingsState = {
  version: "0.1.0",
  release: true,
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
  },
  timezones: ["Europe/Stockholm"],
  activityTypes: [],
  hevy: {
    connected: true,
    status: "Connected",
    apiKeySaved: true,
    enabled: true,
    watchStrategy: "replace",
    matchMode: "automatic",
    titleTemplate: "{clean_title}",
    descriptionTemplate: "{exercises}",
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

test("the Strava dialog reconnects through the official button, linked straight to Strava", async () => {
  startStravaConnect.mockResolvedValue({ url: STRAVA_AUTHORIZE });
  renderConnections();
  const dialog = await openDialog("strava");

  expect(
    await within(dialog).findByRole("link", { name: "Connect with Strava" }),
  ).toHaveAttribute("href", STRAVA_AUTHORIZE);
  expect(startStravaConnect).toHaveBeenCalledTimes(1);
  expect(within(dialog).getByRole("button", { name: "Save credentials" })).toBeDisabled();
  expect(within(dialog).getByRole("button", { name: "Disconnect" })).toBeVisible();
});

// Connecting authorizes against the saved credentials, so unsaved edits
// must be saved before the button links anywhere.
test("editing the Strava credentials disables Connect until they are saved", async () => {
  startStravaConnect.mockResolvedValue({ url: STRAVA_AUTHORIZE });
  saveStravaCredentials.mockResolvedValue({ message: "Strava credentials saved." });
  renderConnections();
  const dialog = await openDialog("strava");
  await within(dialog).findByRole("link", { name: "Connect with Strava" });

  await userEvent.type(within(dialog).getByLabelText("Strava client secret"), "new-secret");

  expect(within(dialog).queryByRole("link", { name: "Connect with Strava" })).toBeNull();
  expect(within(dialog).getByRole("img", { name: "Connect with Strava" })).toBeVisible();
  await userEvent.click(within(dialog).getByRole("button", { name: "Save credentials" }));
  expect(saveStravaCredentials).toHaveBeenCalledWith(
    { clientId: "1234", clientSecret: "new-secret" },
    expect.anything(),
  );
  expect(screen.getByRole("dialog")).toBeVisible();
});

test("a disconnected Strava without saved credentials offers Cancel and a disabled Connect", async () => {
  renderConnections({
    ...baseState,
    connections: {
      ...baseState.connections,
      strava: { connected: false, status: "Not connected", meta: "" },
    },
    credentials: {
      ...baseState.credentials,
      stravaClientId: "",
      stravaClientSecretSaved: false,
    },
  });
  const dialog = await openDialog("strava");

  expect(within(dialog).getByRole("img", { name: "Connect with Strava" })).toBeVisible();
  expect(within(dialog).queryByRole("link", { name: "Connect with Strava" })).toBeNull();
  expect(startStravaConnect).not.toHaveBeenCalled();
  expect(within(dialog).getByRole("button", { name: "Cancel" })).toBeVisible();
  expect(within(dialog).queryByRole("button", { name: "Disconnect" })).toBeNull();
});

// Every settings sheet footer button is a 44px touch target. The header ✕
// that ResponsiveOverlay supplies is what closes the sheet now, so the
// footer holds actions only.
test("every settings dialog footer button is the same height", async () => {
  renderConnections();
  const dialog = await openDialog("garmin-connect");

  // Scoped to the footer: ResponsiveOverlay also renders a header ✕ with an
  // sr-only "Close" label, which is an icon button and deliberately smaller.
  const footer = dialog.querySelector('[data-testid="responsive-overlay-footer"]');
  expect(footer).not.toBeNull();
  const buttons = within(footer as HTMLElement).getAllByRole("button");
  expect(buttons.map((button) => button.textContent)).toEqual(["Reconnect"]);
  for (const button of buttons) {
    expect(button.className).toContain("h-11");
  }
});

// The footer buttons live outside the <form> element (ResponsiveOverlay keeps
// its footer out of the scrolling body), so they reach it by `form=` id.
// Without that wiring, submitting is impossible from the footer at all.
test("the footer's submit button targets the form in the sheet body", async () => {
  renderConnections();
  const dialog = await openDialog("garmin-connect");

  const submit = within(dialog).getByRole("button", { name: "Reconnect" });
  const formId = submit.getAttribute("form");
  expect(formId).toBeTruthy();
  expect(dialog.querySelector(`form#${formId}`)).not.toBeNull();
});

test("a disconnected Hevy dialog asks for a key instead", async () => {
  renderConnections({ ...baseState, hevy: { ...baseState.hevy, connected: false } });
  const dialog = await openDialog("hevy");

  expect(within(dialog).getByLabelText("Hevy API key")).toBeVisible();
  expect(within(dialog).queryByRole("button", { name: "Disconnect" })).toBeNull();
});
