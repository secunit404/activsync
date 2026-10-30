import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { expect, test, vi } from "vitest";

import type { SettingsState } from "@/lib/api";
import { LOGIN_ERRORS } from "@/lib/auth-errors";
import { SettingsView } from "./settings";

const { disconnectHevy } = vi.hoisted(() => ({ disconnectHevy: vi.fn() }));

vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  return { ...actual, disconnectHevy };
});

vi.mock("@/lib/auth-api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/auth-api")>();
  return {
    ...actual,
    getAuthStatus: vi.fn().mockResolvedValue({
      authenticated: true,
      method: "password",
      user: null,
      ownerClaimed: true,
      authDisabled: false,
      login: { oidc: null },
    }),
    getAuthSettings: vi.fn().mockResolvedValue({ proxy: null }),
    getSessions: vi.fn().mockResolvedValue([]),
    getOidcConfig: vi.fn().mockResolvedValue({
      configured: false,
      displayName: "",
      issuer: "",
      clientId: "",
      hasSecret: false,
      scopes: "openid profile email",
      autoRedirect: false,
      redirectUri: "http://localhost/auth/oidc/callback",
      linkedLabel: null,
      logoUrl: null,
    }),
  };
});

// 152 uninteresting placeholders (all fall into the "Other" derived
// category) plus two specifically-named, non-overlapping types so the
// search test can filter unambiguously. 154 total, matching the real
// Garmin taxonomy's size (src/activsync/dev_mock.py's
// GARMIN_ACTIVITY_TYPE_KEYS) without needing the full real list here.
const activityTypes: SettingsState["activityTypes"] = [
  ...Array.from({ length: 152 }, (_, index) => ({
    typeKey: "placeholder-" + index,
    label: "Placeholder " + index,
    autosync: false,
  })),
  { typeKey: "running", label: "Running", autosync: false },
  { typeKey: "swimming", label: "Swimming", autosync: false },
];

const connectedState: SettingsState = {
  version: "0.1.0",
  release: true,
  update: {
    latest: null,
    available: false,
    repoUrl: "https://github.com/secunit404/activsync",
    releaseUrl: "https://github.com/secunit404/activsync/releases/latest",
  },
  development: true,
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
  timezones: ["Europe/Stockholm", "Europe/Oslo"],
  activityTypes,
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
    profileOverride: {
      weightKg: null,
      birthYear: null,
      vo2max: null,
      sex: null,
    },
    profileBaseline: {
      weightKg: 80,
      birthYear: 1990,
      vo2max: 45,
      sex: "male",
    },
    profileFromGarmin: false,
  },
};

function renderWithProviders(node: React.ReactNode) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter>{node}</MemoryRouter>
    </QueryClientProvider>,
  );
}

function renderSettings(state: SettingsState = connectedState) {
  return renderWithProviders(<SettingsView state={state} />);
}

// Settings renders inside the rail, which already shows the brand mark.
// The page header carries no wordmark; only its phone logo mark, which the
// rail's absence below md: calls for. The setup wizard keeps its own
// AppBrand (setup-step-dots.tsx); it has never used this shell.
test("renders no brand bar of its own, since the rail supplies one", () => {
  renderSettings();
  expect(screen.queryByLabelText("ActivSync")).not.toBeInTheDocument();
  const logos = screen.getAllByRole("img", { name: "ActivSync" });
  expect(logos).toHaveLength(1);
  expect(logos[0].closest("header")).not.toBeNull();
  expect(screen.getByRole("heading", { level: 1, name: "Settings" })).toBeVisible();
});

test("the mock-data badge survives in the page header", () => {
  renderSettings({ ...connectedState, development: true });
  expect(screen.getByText("Mock data")).toBeVisible();
});

test("no mock-data badge outside development", () => {
  renderSettings({ ...connectedState, development: false });
  expect(screen.queryByText("Mock data")).toBeNull();
});

test("auto-sync search filters the type list and reports the count", async () => {
  renderSettings();
  expect(await screen.findByText(/All 154/)).toBeInTheDocument();
  await userEvent.type(screen.getByRole("searchbox", { name: /search/i }), "run");
  expect(screen.getByText("Running")).toBeInTheDocument();
  expect(screen.queryByText("Swimming")).not.toBeInTheDocument();
});

test("save is disabled until a section is dirty", async () => {
  renderSettings();
  expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  await userEvent.click(await screen.findByRole("switch", { name: /Running/ }));
  expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
});

test("omits the retired standalone hevy2garmin marker preferences", () => {
  renderSettings();
  expect(
    screen.queryByText(/Auto-publish hevy2garmin imports held by category/i),
  ).toBeNull();
  expect(screen.queryByLabelText(/Marker text/i)).toBeNull();
});

test("renders every section, in the order the design specifies", () => {
  renderSettings();
  const headings = screen
    .getAllByRole("heading", { level: 2 })
    .map((heading) => heading.textContent);
  expect(headings).toEqual([
    "Connections",
    "Access",
    "Preferences",
    "Auto-sync by type",
    "Hevy integration",
  ]);
});

test("renders Strava OAuth failures inside the React settings surface", () => {
  renderWithProviders(
    <SettingsView
      state={connectedState}
      stravaError="Strava authorization was declined."
    />,
  );

  expect(screen.getByText("Strava connection failed")).toBeInTheDocument();
  expect(screen.getByText("Strava authorization was declined.")).toBeInTheDocument();
});

test("an OIDC link failure shows fixed copy for its code", () => {
  renderWithProviders(
    <SettingsView state={connectedState} oidcError="oidc_mismatch" />,
  );

  expect(screen.getByText("Linking single sign-on failed")).toBeInTheDocument();
  expect(screen.getByText(LOGIN_ERRORS.oidc_mismatch)).toBeInTheDocument();
});

test("disables prerequisite actions when services are disconnected", () => {
  renderSettings({
    ...connectedState,
    connections: {
      garmin: {
        ...connectedState.connections.garmin,
        connected: false,
        status: "Disconnected — sync paused",
      },
      strava: {
        ...connectedState.connections.strava,
        connected: false,
        status: "Disconnected — publishing paused",
      },
      broken: ["garmin", "strava"],
    },
  });

  expect(screen.getByRole("button", { name: "Sync Garmin" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Sync Strava" })).toBeDisabled();
  expect(screen.getByRole("button", { name: /Refresh types/ })).toBeDisabled();
});

test("discard reverts a pending preference edit and re-disables Save", async () => {
  renderSettings();
  const timezone = screen.getByLabelText("Display timezone");
  await userEvent.selectOptions(timezone, "Europe/Oslo");
  expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();

  await userEvent.click(screen.getByRole("button", { name: "Discard" }));

  expect(timezone).toHaveValue("Europe/Stockholm");
  expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
});

test("discard also clears the auto-sync search and any toggles it drove", async () => {
  renderSettings();
  await userEvent.type(screen.getByRole("searchbox", { name: /search/i }), "run");
  await userEvent.click(screen.getByRole("switch", { name: /Running/ }));
  expect(screen.getByText("Running")).toBeInTheDocument();

  await userEvent.click(screen.getByRole("button", { name: "Discard" }));

  expect(screen.getByRole("searchbox", { name: /search/i })).toHaveValue("");
  expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
});

test("Hevy automatic strategy is a real radio group, not clickable divs", () => {
  renderSettings();
  const group = screen.getByRole("radiogroup", {
    name: /automatic match strategy/i,
  });
  expect(group).toBeInTheDocument();
  const options = within(group).getAllByRole("radio");
  expect(options.map((option) => option.getAttribute("aria-label"))).toEqual([
    "Replace",
    "Merge",
    "Describe",
  ]);
  expect(screen.getByRole("radio", { name: "Replace" })).toHaveAttribute(
    "data-state",
    "checked",
  );
});

test("Hevy integration omits the obsolete moved-to-hub notice", () => {
  renderSettings();
  expect(screen.queryByText(/moved to/i)).not.toBeInTheDocument();
  expect(screen.queryByRole("link", { name: /Hevy hub/i })).not.toBeInTheDocument();
});

test("disconnect opens a real confirmation dialog with the preserved copy, and cancelling leaves Hevy connected", async () => {
  renderSettings();

  await userEvent.click(
    screen.getAllByRole("button", { name: "Manage" })[2],
  );
  await userEvent.click(screen.getByRole("button", { name: "Disconnect" }));

  const dialog = screen.getByRole("alertdialog", { name: "Disconnect Hevy?" });
  expect(dialog).toHaveAccessibleDescription("This also pauses Hevy sync.");

  await userEvent.click(screen.getByRole("button", { name: "Cancel" }));

  expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
  expect(disconnectHevy).not.toHaveBeenCalled();
  expect(screen.getByRole("button", { name: "Disconnect" })).toBeInTheDocument();
});

test("confirming the dialog disconnects Hevy", async () => {
  disconnectHevy.mockResolvedValue({ message: "Hevy disconnected." });
  renderSettings();

  await userEvent.click(
    screen.getAllByRole("button", { name: "Manage" })[2],
  );
  await userEvent.click(screen.getByRole("button", { name: "Disconnect" }));

  const dialog = screen.getByRole("alertdialog", { name: "Disconnect Hevy?" });
  await userEvent.click(within(dialog).getByRole("button", { name: "Disconnect" }));

  await waitFor(() => expect(disconnectHevy).toHaveBeenCalledTimes(1));
  await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument());
});

function renderSettingsWithRefetch(state: SettingsState) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  const view = (next: SettingsState) => (
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <SettingsView state={next} />
      </MemoryRouter>
    </QueryClientProvider>
  );
  const utils = render(view(state));
  return { refetch: (next: SettingsState) => utils.rerender(view(next)) };
}

test("a refetch keeps unsaved edits and follows the server for untouched fields", async () => {
  const { refetch } = renderSettingsWithRefetch(connectedState);
  const garmin = screen.getByLabelText("Garmin sync interval (min)");
  await userEvent.clear(garmin);
  await userEvent.type(garmin, "45");

  refetch({
    ...connectedState,
    preferences: { ...connectedState.preferences, lookbackDays: 14 },
  });

  expect(screen.getByLabelText("Garmin sync interval (min)")).toHaveValue(45);
  expect(screen.getByLabelText("Activity history window (days)")).toHaveValue(14);
  expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
  expect(screen.queryByText(/changed on the server/i)).toBeNull();
});

test("a refetch that moves an edited field keeps the draft and says so", async () => {
  const { refetch } = renderSettingsWithRefetch(connectedState);
  const garmin = screen.getByLabelText("Garmin sync interval (min)");
  await userEvent.clear(garmin);
  await userEvent.type(garmin, "45");

  refetch({
    ...connectedState,
    preferences: { ...connectedState.preferences, garminPollIntervalMinutes: 30 },
  });

  expect(screen.getByLabelText("Garmin sync interval (min)")).toHaveValue(45);
  expect(screen.getByText(/changed on the server while you were editing/i)).toBeVisible();

  await userEvent.click(screen.getByRole("button", { name: "Discard" }));
  expect(screen.getByLabelText("Garmin sync interval (min)")).toHaveValue(30);
  expect(screen.queryByText(/changed on the server/i)).toBeNull();
});

test("an activity type edit survives a refetch", async () => {
  const { refetch } = renderSettingsWithRefetch(connectedState);
  await userEvent.click(screen.getByRole("switch", { name: /Running/ }));

  refetch({ ...connectedState, version: "0.1.1" });

  expect(screen.getByRole("switch", { name: /Running/ })).toBeChecked();
  expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
});

test("once the server reflects the saved edit nothing stays dirty", async () => {
  const { refetch } = renderSettingsWithRefetch(connectedState);
  const garmin = screen.getByLabelText("Garmin sync interval (min)");
  await userEvent.clear(garmin);
  await userEvent.type(garmin, "45");

  refetch({
    ...connectedState,
    preferences: { ...connectedState.preferences, garminPollIntervalMinutes: 45 },
  });

  expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  expect(screen.queryByText(/changed on the server/i)).toBeNull();
});
