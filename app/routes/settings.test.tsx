import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { expect, test } from "vitest";

import type { SettingsState } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import { SettingsView } from "./settings";
import { SetupView } from "./setup";

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
  activityTypes: Array.from({ length: 20 }, (_, index) => ({
    typeKey: "type-" + index,
    label: "Category " + index,
    autosync: index < 3,
  })),
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
  client.setQueryData(queryKeys.hevyTools, {
    mappings: [],
    categories: [],
  });
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter>{node}</MemoryRouter>
    </QueryClientProvider>,
  );
}

test("renders the complete mobile-first settings surface", () => {
  renderWithProviders(<SettingsView state={connectedState} />);

  expect(screen.getByRole("heading", { name: "Settings" })).toBeInTheDocument();
  expect(screen.getByRole("heading", { name: "Connections" })).toBeInTheDocument();
  expect(
    screen.getByRole("heading", { name: "Autosync categories" }),
  ).toBeInTheDocument();
  expect(screen.getByRole("heading", { name: "Preferences" })).toBeInTheDocument();
  expect(screen.getByRole("heading", { name: "Hevy" })).toBeInTheDocument();
  expect(screen.getByRole("status")).toHaveTextContent("3 enabled · 18 shown");
  expect(
    screen.getByRole("button", { name: "Show all 20 categories" }),
  ).toHaveAttribute("aria-expanded", "false");
  expect(screen.getByText("Mock data")).toBeInTheDocument();
  expect(screen.getByText("ActivSync v0.1.0")).toBeInTheDocument();
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
  renderWithProviders(
    <SettingsView
      state={{
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
      }}
    />,
  );

  expect(screen.getByRole("button", { name: "Sync Garmin" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Sync Strava" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Save categories" })).toBeDisabled();
});

test("renders Garmin setup with the accepted history-window wording", () => {
  renderWithProviders(
    <SetupView
      state={{
        ...connectedState,
        setup: { complete: false, step: "garmin", mfaRequired: false },
        connections: {
          ...connectedState.connections,
          garmin: {
            ...connectedState.connections.garmin,
            connected: false,
            status: "Disconnected — sync paused",
          },
          broken: ["garmin"],
        },
        credentials: {
          ...connectedState.credentials,
          garminEmail: "",
          garminPasswordSaved: false,
        },
      }}
    />,
  );

  expect(
    screen.getByRole("heading", { name: "Connect Garmin" }),
  ).toBeInTheDocument();
  expect(
    screen.getByLabelText("Activity history window (days)"),
  ).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Connect Garmin" })).toBeEnabled();
});

test("renders the Garmin MFA escape hatch", () => {
  renderWithProviders(
    <SetupView
      state={{
        ...connectedState,
        setup: { complete: false, step: "garmin", mfaRequired: true },
      }}
    />,
  );

  expect(
    screen.getByRole("heading", { name: "Verify Garmin" }),
  ).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Cancel" })).toBeInTheDocument();
});

test("renders Hevy mapping editors from the typed tools query", () => {
  const client = new QueryClient();
  client.setQueryData(queryKeys.hevyTools, {
    mappings: [
      {
        templateId: "custom-1",
        title: "Landmine Press",
        isCustom: true,
        muscleGroup: "chest",
        mapped: false,
        unmapped: true,
        garminRejected: false,
        suggested: true,
        category: 0,
        subcategory: 1,
        categoryName: "Bench Press",
        subcategoryName: "Barbell Bench Press",
      },
    ],
    categories: [
      {
        value: 0,
        label: "Bench Press",
        subcategories: [{ value: 1, label: "Barbell Bench Press" }],
      },
    ],
  });

  render(
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <SettingsView state={connectedState} />
      </MemoryRouter>
    </QueryClientProvider>,
  );

  expect(screen.getByRole("heading", { name: "Exercise mappings" })).toBeInTheDocument();
  expect(screen.getByRole("heading", { name: "Landmine Press" })).toBeInTheDocument();
  expect(screen.getByText("Needs mapping")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Save mapping" })).toBeEnabled();
});
