export type SetupStep = "garmin" | "strava" | "hevy" | "syncing";
export type PublishStatus =
  | "pending"
  | "held"
  | "published"
  | "missing"
  | "excluded";
export type SortOrder = "newest" | "oldest";
export type PageSize = 10 | 20 | 50 | 100;

export type UpdateState = {
  latest: string | null;
  available: boolean;
  repoUrl: string;
  releaseUrl: string;
};

export type ServiceConnection = {
  connected: boolean;
  status: string;
  meta: string;
};

export type AppState = {
  name: "ActivSync";
  version: string;
  development: boolean;
  setup: {
    complete: boolean;
    step: SetupStep | null;
  };
  connections: {
    garmin: ServiceConnection & { email: string };
    strava: ServiceConnection;
    broken: Array<"garmin" | "strava">;
  };
  hevy: {
    enabled: boolean;
    connected: boolean;
    status: string;
  };
  catchUpReport: {
    new: number;
    held: number;
    linked: number;
    days: number;
  } | null;
  update: UpdateState;
};

export type ActivityDetail = {
  description: string | null;
  distance: string;
  duration: string;
  movingTime: string;
  elapsedTime: string;
  pace: string;
  speed: string;
  elevGain: string;
  elevLoss: string;
  calories: string;
  avgHr: string;
  maxHr: string;
  avgPower: string;
  maxPower: string;
  normPower: string;
  aerobicTe: string;
  anaerobicTe: string;
  trainingLoad: string;
  avgCadence: string;
  maxCadence: string;
  totalSets: string;
  totalReps: string;
  totalVolume: string;
};

export type Activity = {
  garminActivityId: number;
  activityType: string;
  title: string;
  description: string;
  startTime: string;
  publishStatus: PublishStatus;
  stravaActivityId: number | null;
  holdReason: string | null;
  startDateDisplay: string;
  startMonthYearDisplay: string;
  startClockDisplay: string;
  garminUrl: string;
  stravaUrl: string | null;
  hevyBadge: string | null;
  detail: ActivityDetail;
};

export type ActivityQuery = {
  sort: SortOrder;
  status: PublishStatus | null;
  page: number;
  pageSize: PageSize;
};

export type WeekTotal = {
  seconds: number;
  display: string;
};

export type ActivitiesPage = {
  items: Activity[];
  sort: SortOrder;
  status: PublishStatus | null;
  counts: Record<PublishStatus, number>;
  weekTotal: WeekTotal;
  pagination: {
    page: number;
    pageSize: PageSize;
    pageCount: number;
    totalCount: number;
    firstItem: number;
    lastItem: number;
  };
};

export type ActivityActionResult = {
  message: string;
  severity: "success" | "warning";
  publishedCount: number;
  failedCount: number;
  blockedCount: number;
};

export type HevyWorkoutSetDetail = {
  number: number;
  setType: string;
  reps: number | null;
  weightKg: number | null;
  distanceMeters: number | null;
  durationSeconds: number | null;
  rpe: number | null;
  customMetric: string | number | null;
};

export type HevyWorkoutExerciseDetail = {
  title: string;
  notes: string | null;
  templateId: string | null;
  sets: HevyWorkoutSetDetail[];
};

export type HevyWorkoutDetail = {
  hevyId: string;
  title: string;
  startTime: string;
  endTime: string;
  notes: string | null;
  exercises: HevyWorkoutExerciseDetail[];
  descriptionPreview: string;
};

export type SettingsState = {
  version: string;
  update: UpdateState;
  development: boolean;
  setup: {
    complete: boolean;
    step: SetupStep | null;
    mfaRequired: boolean;
  };
  connections: AppState["connections"];
  credentials: {
    garminEmail: string;
    garminPasswordSaved: boolean;
    stravaClientId: string;
    stravaClientSecretSaved: boolean;
  };
  preferences: {
    displayTimezone: string;
    garminPollIntervalMinutes: number;
    stravaPollIntervalMinutes: number;
    lookbackDays: number;
  };
  timezones: string[];
  activityTypes: Array<{
    typeKey: string;
    label: string;
    autosync: boolean;
  }>;
  hevy: {
    connected: boolean;
    status: string;
    apiKeySaved: boolean;
    enabled: boolean;
    watchStrategy: "replace" | "merge" | "describe";
    matchMode: "review" | "automatic";
    descriptionTemplate: string;
    summaryOnStructured: boolean;
    graceMinutes: number;
    pollIntervalMinutes: number;
    identity: {
      manufacturer: number | null;
      product: number | null;
      serial: number | null;
    };
    identityDisplay: string;
    profileOverride: {
      weightKg: number | null;
      birthYear: number | null;
      vo2max: number | null;
      sex: "male" | "female" | null;
    };
    profileBaseline: {
      weightKg: number;
      birthYear: number;
      vo2max: number;
      sex: string;
    };
    profileFromGarmin: boolean;
  };
};

export type SettingsActionResult = {
  message: string;
  setupStep: SetupStep | null;
  mfaRequired: boolean;
};

export type HevyToolsState = {
  mappings: Array<{
    templateId: string;
    title: string;
    isCustom: boolean;
    muscleGroup: string;
    mapped: boolean;
    unmapped: boolean;
    suggested: boolean;
    /**
     * Who chose the Garmin pair: `"user"` for a saved override, `"automatic"`
     * for one the ported tables resolve, `""` when nothing resolves it yet.
     */
    source: "user" | "automatic" | "";
    hasStandardMapping: boolean;
    standardCategory: number | null;
    standardSubcategory: number | null;
    category: number | null;
    subcategory: number | null;
    categoryName: string | null;
    subcategoryName: string | null;
  }>;
  categories: Array<{
    value: number;
    label: string;
    subcategories: Array<{ value: number; label: string }>;
  }>;
};

export type BackfillResult = {
  message: string;
  since: string;
  ran: boolean;
  linked: number;
  items: Array<{
    hevyId: string;
    title: string;
    startTime: string;
    action: string;
    twinActivityId: number | null;
    garminUrl: string | null;
    stravaActivityId: number | null;
    stravaUrl: string | null;
    missingTemplateIds: string[];
    workout: HevyWorkoutDetail;
  }>;
};

export type HevyQueueItem = {
  hevyId: string;
  title: string;
  status: string;
  error: string | null;
  startDisplay: string;
  needsMapping: boolean;
  hasOpenOperation: boolean;
  resyncable: boolean;
  awaitingMatch: boolean;
  matchedGarminActivityId: number | null;
  matchedGarminTitle: string | null;
  matchedStravaActivityId: number | null;
  matchedStravaUrl: string | null;
};

export type HevyQueueState = {
  enabled: boolean;
  inFlight: HevyQueueItem[];
  problems: HevyQueueItem[];
  skipped: HevyQueueItem[];
  counts: {
    inFlight: number;
    problems: number;
    skipped: number;
  };
};

export type HevyQueueAction = "retry" | "skip" | "unskip" | "resync-fresh";
export type HevyMatchStrategy = "merge" | "replace" | "describe";

/** One entry in a FIT profile enum — the integer is what gets stored in
 *  `hevy_device_identity`; the label is display only. */
export type DeviceOption = { value: number; label: string };

export type DeviceOptions = {
  manufacturers: DeviceOption[];
  products: DeviceOption[];
};

export class ApiError extends Error {
  constructor(
    message: string,
    readonly status: number,
  ) {
    super(message);
    this.name = "ApiError";
  }
}

export function getAppState(signal?: AbortSignal): Promise<AppState> {
  return requestJson<AppState>("/api/v1/app", { signal });
}

export async function getActivities(
  query: ActivityQuery,
  signal?: AbortSignal,
): Promise<ActivitiesPage> {
  const search = new URLSearchParams({
    sort: query.sort,
    page: String(query.page),
    pageSize: String(query.pageSize),
  });
  if (query.status !== null) {
    search.set("status", query.status);
  }

  return requestJson<ActivitiesPage>(`/api/v1/activities?${search}`, { signal });
}

export function dismissCatchUpReport() {
  return requestJson<void>("/api/v1/catch-up-report", { method: "DELETE" });
}

export function getSettings(signal?: AbortSignal): Promise<SettingsState> {
  return requestJson<SettingsState>("/api/v1/settings", { signal });
}

export function setupGarmin(payload: {
  email: string;
  password: string;
  lookbackDays: number;
  detectedTimezone: string;
}) {
  return requestJson<SettingsActionResult>("/api/v1/setup/garmin", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function reconnectGarmin(payload: { email: string; password: string }) {
  return requestJson<SettingsActionResult>(
    "/api/v1/settings/garmin/reconnect",
    { method: "POST", body: JSON.stringify(payload) },
  );
}

export function completeGarminMfa(code: string) {
  return requestJson<SettingsActionResult>("/api/v1/garmin/mfa", {
    method: "POST",
    body: JSON.stringify({ code }),
  });
}

export function cancelGarminMfa() {
  return requestJson<SettingsActionResult>("/api/v1/garmin/mfa", {
    method: "DELETE",
  });
}

export function saveStravaCredentials(payload: {
  clientId: string;
  clientSecret: string;
}) {
  return requestJson<SettingsActionResult>(
    "/api/v1/settings/strava-credentials",
    { method: "PUT", body: JSON.stringify(payload) },
  );
}

export function setupHevy(apiKey: string) {
  return requestJson<SettingsActionResult>("/api/v1/setup/hevy", {
    method: "POST",
    body: JSON.stringify({ apiKey }),
  });
}

export function skipHevy() {
  return requestJson<SettingsActionResult>("/api/v1/setup/hevy/skip", {
    method: "POST",
  });
}

export function runInitialSync() {
  return requestJson<SettingsActionResult>("/api/v1/setup/initial-sync", {
    method: "POST",
  });
}

export function savePreferences(payload: SettingsState["preferences"]) {
  return requestJson<SettingsActionResult>("/api/v1/settings/preferences", {
    method: "PUT",
    body: JSON.stringify(payload),
  });
}

export function saveActivityTypes(autosyncTypes: string[]) {
  return requestJson<SettingsActionResult>("/api/v1/settings/activity-types", {
    method: "PUT",
    body: JSON.stringify({ autosyncTypes }),
  });
}

export function refreshActivityTypes() {
  return requestJson<SettingsActionResult>(
    "/api/v1/settings/activity-types/refresh",
    { method: "POST" },
  );
}

export function runManualSync(service: "garmin" | "strava") {
  return requestJson<SettingsActionResult>(`/api/v1/settings/sync/${service}`, {
    method: "POST",
  });
}

export function saveHevyCredentials(apiKey: string) {
  return requestJson<SettingsActionResult>(
    "/api/v1/settings/hevy/credentials",
    { method: "POST", body: JSON.stringify({ apiKey }) },
  );
}

export function saveHevySettings(payload: {
  enabled: boolean;
  watchStrategy: SettingsState["hevy"]["watchStrategy"];
  matchMode: SettingsState["hevy"]["matchMode"];
  descriptionTemplate: string;
  summaryOnStructured: boolean;
  graceMinutes: number;
  pollIntervalMinutes: number;
  identity: SettingsState["hevy"]["identity"];
  profileOverride: SettingsState["hevy"]["profileOverride"];
}) {
  return requestJson<SettingsActionResult>("/api/v1/settings/hevy", {
    method: "PUT",
    body: JSON.stringify(payload),
  });
}

export function disconnectHevy() {
  return requestJson<SettingsActionResult>("/api/v1/settings/hevy", {
    method: "DELETE",
  });
}

export function disconnectStrava() {
  return requestJson<SettingsActionResult>("/api/v1/settings/strava", {
    method: "DELETE",
  });
}

export function getHevyTools(signal?: AbortSignal): Promise<HevyToolsState> {
  return requestJson<HevyToolsState>("/api/v1/settings/hevy/tools", { signal });
}

export function getHevyDeviceOptions(signal?: AbortSignal): Promise<DeviceOptions> {
  return requestJson<DeviceOptions>("/api/v1/settings/hevy/device-options", {
    signal,
  });
}

export function saveExerciseMapping(
  templateId: string,
  category: number,
  subcategory: number,
) {
  return requestJson<{ message: string }>(
    "/api/v1/settings/hevy/mappings/" + encodeURIComponent(templateId),
    {
      method: "PUT",
      body: JSON.stringify({ category, subcategory }),
    },
  );
}

export function removeExerciseMapping(templateId: string) {
  return requestJson<{ message: string }>(
    "/api/v1/settings/hevy/mappings/" + encodeURIComponent(templateId),
    { method: "DELETE" },
  );
}

export function previewHevyBackfill(since: string) {
  return requestJson<BackfillResult>(
    "/api/v1/settings/hevy/backfill/preview",
    { method: "POST", body: JSON.stringify({ since }) },
  );
}

export function runHevyBackfill(since: string, hevyIds: string[]) {
  return requestJson<BackfillResult>("/api/v1/settings/hevy/backfill/run", {
    method: "POST",
    body: JSON.stringify({ since, hevyIds }),
  });
}

export function getHevyQueue(signal?: AbortSignal): Promise<HevyQueueState> {
  return requestJson<HevyQueueState>("/api/v1/hevy/queue", { signal });
}

export function getHevyWorkout(
  hevyId: string,
  signal?: AbortSignal,
): Promise<HevyWorkoutDetail> {
  return requestJson<HevyWorkoutDetail>(
    `/api/v1/hevy/${encodeURIComponent(hevyId)}`,
    { signal },
  );
}

export function runHevyQueueAction(hevyId: string, action: HevyQueueAction) {
  return requestJson<{ message: string }>(
    `/api/v1/hevy/${encodeURIComponent(hevyId)}/${action}`,
    { method: "POST" },
  );
}

export function chooseHevyMatch(hevyId: string, strategy: HevyMatchStrategy) {
  return requestJson<{ message: string }>(
    `/api/v1/hevy/${encodeURIComponent(hevyId)}/match`,
    { method: "POST", body: JSON.stringify({ strategy }) },
  );
}

export function publishActivity(garminActivityId: number) {
  return requestJson<ActivityActionResult>(
    `/api/v1/activities/${garminActivityId}/publish`,
    { method: "POST" },
  );
}

export function publishActivities(activityIds: number[]) {
  return requestJson<ActivityActionResult>("/api/v1/activities/publish", {
    method: "POST",
    body: JSON.stringify({ activityIds }),
  });
}

export function editActivity(
  garminActivityId: number,
  title: string,
  description: string,
) {
  return requestJson<ActivityActionResult>(
    `/api/v1/activities/${garminActivityId}`,
    {
      method: "PUT",
      body: JSON.stringify({ title, description }),
    },
  );
}

export function excludeActivity(garminActivityId: number) {
  return requestJson<ActivityActionResult>(
    `/api/v1/activities/${garminActivityId}/exclude`,
    { method: "POST" },
  );
}

export function restoreActivity(garminActivityId: number) {
  return requestJson<ActivityActionResult>(
    `/api/v1/activities/${garminActivityId}/restore`,
    { method: "POST" },
  );
}

async function requestJson<T>(url: string, init: RequestInit): Promise<T> {
  const response = await fetch(url, {
    ...init,
    headers: {
      Accept: "application/json",
      "Content-Type": "application/json",
      ...init.headers,
    },
  });
  if (response.status === 204) {
    return undefined as T;
  }
  const payload = (await response.json().catch(() => null)) as {
    detail?: string;
  } | null;
  if (!response.ok) {
    throw new ApiError(
      payload?.detail ?? `ActivSync request failed (${response.status})`,
      response.status,
    );
  }
  return payload as T;
}
