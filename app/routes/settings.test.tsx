import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { expect, test, vi } from "vitest";

import type { SettingsState } from "@/lib/api";
import { SettingsView } from "./settings";

const { disconnectHevy } = vi.hoisted(() => ({ disconnectHevy: vi.fn() }));

vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  return { ...actual, disconnectHevy };
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
    hevy2garminMarker: "— synced by hevy2garmin",
    hevy2garminMarkerEnabled: false,
  },
  timezones: ["Europe/Stockholm", "Europe/Oslo"],
  activityTypes,
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
    profileOverride: {
      weightKg: null,
      birthYear: null,
      vo2max: null,
      sex: null,
    },
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

test("auto-sync search filters the type list and reports the count", async () => {
  renderSettings();
  expect(await screen.findByText(/of 154/)).toBeInTheDocument();
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

test("renders every section, in the order the design specifies", () => {
  renderSettings();
  const headings = screen
    .getAllByRole("heading", { level: 2 })
    .map((heading) => heading.textContent);
  expect(headings).toEqual([
    "Connections",
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

test("Hevy watch strategy is a real radio group, not clickable divs", () => {
  renderSettings();
  const group = screen.getByRole("radiogroup", { name: /watch strategy/i });
  expect(group).toBeInTheDocument();
  const options = screen.getAllByRole("radio");
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

test("Hevy integration links out to the Hevy hub for operations", () => {
  renderSettings();
  expect(screen.getByRole("link", { name: /Hevy hub/i })).toHaveAttribute(
    "href",
    "/hevy",
  );
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
