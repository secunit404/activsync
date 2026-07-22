import { useQuery } from "@tanstack/react-query";
import { CheckIcon } from "lucide-react";

import { SettingsSection } from "@/components/settings-shell";
import {
  Field,
  FieldDescription,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import {
  NativeSelect,
  NativeSelectOption,
} from "@/components/ui/native-select";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import type { HevyDraft } from "@/lib/hevy-draft";
import { renderDescriptionPreview } from "@/lib/hevy-description";
import { getHevyDeviceOptions, type SettingsState } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import { cn } from "@/lib/utils";

const STRATEGIES: Array<{
  value: SettingsState["hevy"]["watchStrategy"];
  label: string;
  description: string;
}> = [
  {
    value: "replace",
    label: "Replace",
    description:
      "Overwrites the Garmin activity's exercise data with the Hevy workout. Best when Hevy is your source of truth for lifts.",
  },
  {
    value: "merge",
    label: "Merge",
    description:
      "Keeps existing Garmin data and adds only exercises that Hevy has and Garmin is missing.",
  },
  {
    value: "describe",
    label: "Describe",
    description:
      "Leaves Garmin untouched and just appends a text summary of the Hevy workout to the description.",
  },
];

const MATCH_MODES: Array<{
  value: SettingsState["hevy"]["matchMode"];
  label: string;
  description: string;
}> = [
  {
    value: "review",
    label: "Review each match",
    description:
      "Pauses when a Hevy workout matches Garmin so you choose Merge, Replace, or Description only.",
  },
  {
    value: "automatic",
    label: "Automatic",
    description:
      "Applies the automatic strategy below as soon as ActivSync finds one Garmin match.",
  },
];

/**
 * Hevy *configuration* only — connect/disconnect lives in the Connections
 * section (settings-connections.tsx) since that's the account-credential
 * concern. Operations (sync queue, exercise mapping, backfill) moved out of
 * Settings entirely to the /hevy hub (Tasks 14-16); this section links
 * there rather than embedding them, matching the IA change for Task 12.
 */
export function HevySettings({
  state,
  draft,
  onChange,
}: {
  state: SettingsState;
  draft: HevyDraft;
  onChange: (next: HevyDraft) => void;
}) {
  function update(patch: Partial<HevyDraft>) {
    onChange({ ...draft, ...patch });
  }
  const descriptionPreview = renderDescriptionPreview(draft.descriptionTemplate);

  // Reference data — no DB read and no Hevy key needed server-side, so it
  // is safe to fetch whenever this section renders. `staleTime: Infinity`
  // because FIT profile enums cannot change without a redeploy.
  const deviceOptions = useQuery({
    queryKey: queryKeys.hevyDeviceOptions,
    queryFn: ({ signal }) => getHevyDeviceOptions(signal),
    staleTime: Infinity,
  });

  function updateIdentity(patch: Partial<HevyDraft["identity"]>) {
    update({ identity: { ...draft.identity, ...patch } });
  }

  return (
    <SettingsSection
      id="hevy"
      title="Hevy integration"
      description="Bring strength workouts from Hevy into Garmin, then publish the reviewed result to Strava."
      action={
        state.hevy.connected ? (
          <span className="flex items-center gap-2.5">
            <span className="font-mono text-xs text-primary">
              {draft.enabled ? "ENABLED" : "DISABLED"}
            </span>
            <Switch
              id="hevy-enabled"
              aria-label="Enable Hevy sync"
              checked={draft.enabled}
              onCheckedChange={(checked) => update({ enabled: checked === true })}
            />
          </span>
        ) : undefined
      }
    >
      {!state.hevy.connected ? (
        <p className="rounded-lg border border-dashed border-border/70 p-4 text-sm text-muted-foreground">
          Connect Hevy from Connections above to configure watch strategy and
          sync behavior.
        </p>
      ) : (
        <div className="grid gap-6">
          <div className="grid gap-2.5">
            <p
              id="hevy-match-mode-label"
              className="font-mono text-xs font-semibold tracking-[.06em] text-muted-foreground uppercase"
            >
              Match handling
            </p>
            <RadioGroup
              aria-labelledby="hevy-match-mode-label"
              className="grid gap-2.5 sm:grid-cols-2"
              value={draft.matchMode}
              onValueChange={(value) =>
                update({
                  matchMode: value as SettingsState["hevy"]["matchMode"],
                })
              }
            >
              {MATCH_MODES.map((mode) => {
                const active = draft.matchMode === mode.value;
                return (
                  <RadioGroupItem
                    key={mode.value}
                    value={mode.value}
                    aria-label={mode.label}
                    className={cn(
                      "flex flex-col gap-1.5 border p-3.5",
                      active
                        ? "border-primary/60 bg-primary/[0.06]"
                        : "border-border/70",
                    )}
                  >
                    <span className="flex items-center justify-between gap-2">
                      <span
                        className={cn(
                          "text-sm font-bold",
                          active ? "text-primary" : "text-foreground",
                        )}
                      >
                        {mode.label}
                      </span>
                      <span
                        aria-hidden="true"
                        className={cn(
                          "grid size-4 shrink-0 place-items-center rounded-full",
                          active
                            ? "bg-primary text-primary-foreground"
                            : "border border-input",
                        )}
                      >
                        {active ? <CheckIcon className="size-2.5" /> : null}
                      </span>
                    </span>
                    <span className="text-xs leading-relaxed text-muted-foreground">
                      {mode.description}
                    </span>
                  </RadioGroupItem>
                );
              })}
            </RadioGroup>
          </div>

          <div className="grid gap-2.5">
            <p
              id="hevy-strategy-label"
              className="font-mono text-xs font-semibold tracking-[.06em] text-muted-foreground uppercase"
            >
              Automatic match strategy
            </p>
            <RadioGroup
              aria-labelledby="hevy-strategy-label"
              className="grid gap-2.5 sm:grid-cols-3"
              value={draft.watchStrategy}
              onValueChange={(value) =>
                update({
                  watchStrategy: value as SettingsState["hevy"]["watchStrategy"],
                })
              }
            >
              {STRATEGIES.map((strategy) => {
                const active = draft.watchStrategy === strategy.value;
                return (
                  <RadioGroupItem
                    key={strategy.value}
                    value={strategy.value}
                    aria-label={strategy.label}
                    className={cn(
                      "flex flex-col gap-1.5 border p-3.5",
                      active
                        ? "border-primary/60 bg-primary/[0.06]"
                        : "border-border/70",
                    )}
                  >
                    <span className="flex items-center justify-between gap-2">
                      <span
                        className={cn(
                          "text-sm font-bold",
                          active ? "text-primary" : "text-foreground",
                        )}
                      >
                        {strategy.label}
                      </span>
                      <span
                        aria-hidden="true"
                        className={cn(
                          "grid size-4 shrink-0 place-items-center rounded-full",
                          active
                            ? "bg-primary text-primary-foreground"
                            : "border border-input",
                        )}
                      >
                        {active ? <CheckIcon className="size-2.5" /> : null}
                      </span>
                    </span>
                    <span className="text-xs leading-relaxed text-muted-foreground">
                      {strategy.description}
                    </span>
                  </RadioGroupItem>
                );
              })}
            </RadioGroup>
          </div>

          <div className="grid gap-4 rounded-xl border border-border/70 bg-muted/20 p-4">
            <div className="flex items-start justify-between gap-4">
              <div className="grid gap-1">
                <FieldLabel htmlFor="hevy-summary-on-structured">
                  Write summary for Merge and Replace
                </FieldLabel>
                <FieldDescription>
                  Description only always writes the summary. Turn this off to
                  keep the Garmin description unchanged for structured matches.
                </FieldDescription>
              </div>
              <Switch
                id="hevy-summary-on-structured"
                className="mt-0.5 shrink-0"
                checked={draft.summaryOnStructured}
                onCheckedChange={(checked) =>
                  update({ summaryOnStructured: checked === true })
                }
              />
            </div>

            <div className="grid gap-4 lg:grid-cols-2 lg:items-start">
              <Field className="gap-2">
                <div className="flex min-h-5 items-center">
                  <FieldLabel htmlFor="hevy-description-template">
                    Description template
                  </FieldLabel>
                </div>
                <Textarea
                  id="hevy-description-template"
                  className="min-h-56 font-mono text-sm"
                  value={draft.descriptionTemplate}
                  maxLength={4000}
                  aria-invalid={descriptionPreview.error ? true : undefined}
                  onChange={(event) =>
                    update({ descriptionTemplate: event.target.value })
                  }
                  required
                />
                <FieldDescription className="text-xs leading-relaxed">
                  Available placeholders: {"{title}"}, {"{clean_title}"},{" "}
                  {"{duration}"}, {"{calories}"}, {"{avg_hr}"}, {"{exercises}"},
                  and {"{marker}"}. {"{clean_title}"} removes emoji from the Hevy title.
                  Empty metrics are removed automatically. Use {"{{"} and {"}}"} for
                  literal braces.
                </FieldDescription>
              </Field>

              <div className="flex min-w-0 flex-col gap-2">
                <div className="flex min-h-5 items-center justify-between gap-3">
                  <h3
                    id="hevy-description-preview-title"
                    className="text-sm font-medium"
                  >
                    Plain-text preview
                  </h3>
                  <span className="rounded-md bg-muted px-2 py-0.5 font-mono text-[10px] text-muted-foreground uppercase">
                    Sample data
                  </span>
                </div>

                <section
                  aria-labelledby="hevy-description-preview-title"
                  className="flex min-h-56 flex-col rounded-lg border border-border/70 bg-card p-4"
                >
                  {descriptionPreview.error ? (
                    <p role="alert" className="mb-3 text-xs text-destructive">
                      {descriptionPreview.error}
                    </p>
                  ) : null}
                  <p className="flex-1 whitespace-pre-wrap text-sm leading-relaxed text-muted-foreground">
                    {descriptionPreview.text || "The description is empty."}
                  </p>
                </section>

                <p className="text-xs leading-relaxed text-muted-foreground">
                  Garmin and Strava treat this as plain text. Line breaks,
                  emoji, Unicode symbols, and bullets work.
                </p>
              </div>
            </div>
          </div>

          <FieldGroup className="grid gap-5 sm:grid-cols-2 sm:max-w-md">
            <Field>
              <FieldLabel htmlFor="hevy-grace">Grace period (min)</FieldLabel>
              <Input
                id="hevy-grace"
                className="h-11"
                type="number"
                min={0}
                max={1440}
                value={draft.graceMinutes}
                onChange={(event) =>
                  update({ graceMinutes: event.target.valueAsNumber })
                }
                required
              />
            </Field>
            <Field>
              <FieldLabel htmlFor="hevy-poll">Poll interval (min)</FieldLabel>
              <Input
                id="hevy-poll"
                className="h-11"
                type="number"
                min={1}
                max={120}
                value={draft.pollIntervalMinutes}
                onChange={(event) =>
                  update({ pollIntervalMinutes: event.target.valueAsNumber })
                }
                required
              />
            </Field>
          </FieldGroup>

          <details className="rounded-xl border border-border/70 bg-muted/20 p-4">
            <summary className="min-h-8 cursor-pointer font-semibold">
              Device and profile overrides
            </summary>
            <div className="mt-5 grid gap-6">
              <div className="grid gap-2">
                <p className="text-sm font-medium">Device identity</p>
                <p className="text-sm text-muted-foreground">
                  {state.hevy.identityDisplay}. Enter all three values or leave
                  all blank for automatic detection.
                </p>
                <FieldGroup className="grid gap-4 sm:grid-cols-3">
                  <Field>
                    <FieldLabel htmlFor="identity-manufacturer">
                      Manufacturer
                    </FieldLabel>
                    <NativeSelect
                      id="identity-manufacturer"
                      className="w-full"
                      value={draft.identity.manufacturer}
                      onChange={(event) =>
                        updateIdentity({ manufacturer: event.target.value })
                      }
                    >
                      <NativeSelectOption value="">Automatic</NativeSelectOption>
                      {(deviceOptions.data?.manufacturers ?? []).map((option) => (
                        <NativeSelectOption
                          key={option.value}
                          value={String(option.value)}
                        >
                          {option.label}
                        </NativeSelectOption>
                      ))}
                    </NativeSelect>
                  </Field>
                  <Field>
                    <FieldLabel htmlFor="identity-product">Product</FieldLabel>
                    <NativeSelect
                      id="identity-product"
                      className="w-full"
                      value={draft.identity.product}
                      onChange={(event) =>
                        updateIdentity({ product: event.target.value })
                      }
                    >
                      <NativeSelectOption value="">Automatic</NativeSelectOption>
                      {(deviceOptions.data?.products ?? []).map((option) => (
                        <NativeSelectOption
                          key={option.value}
                          value={String(option.value)}
                        >
                          {option.label}
                        </NativeSelectOption>
                      ))}
                    </NativeSelect>
                  </Field>
                  <Field>
                    <FieldLabel htmlFor="identity-serial">Serial</FieldLabel>
                    {/* Per-device and unguessable — there is no list to
                        offer, so this stays a manual entry. */}
                    <Input
                      id="identity-serial"
                      className="h-11"
                      type="number"
                      value={draft.identity.serial}
                      onChange={(event) =>
                        updateIdentity({ serial: event.target.value })
                      }
                    />
                  </Field>
                </FieldGroup>
              </div>
              <div className="grid gap-2">
                <p className="text-sm font-medium">Profile override</p>
                <FieldGroup className="grid gap-4 sm:grid-cols-2">
                  <OptionalNumber
                    id="profile-weight"
                    label="Weight (kg)"
                    value={draft.profile.weightKg}
                    onChange={(value) =>
                      update({ profile: { ...draft.profile, weightKg: value } })
                    }
                  />
                  <OptionalNumber
                    id="profile-birth"
                    label="Birth year"
                    value={draft.profile.birthYear}
                    onChange={(value) =>
                      update({ profile: { ...draft.profile, birthYear: value } })
                    }
                  />
                  <OptionalNumber
                    id="profile-vo2"
                    label="VO₂ max"
                    value={draft.profile.vo2max}
                    onChange={(value) =>
                      update({ profile: { ...draft.profile, vo2max: value } })
                    }
                  />
                  <Field>
                    <FieldLabel htmlFor="profile-sex">Sex</FieldLabel>
                    <NativeSelect
                      id="profile-sex"
                      className="w-full"
                      value={draft.profile.sex}
                      onChange={(event) =>
                        update({
                          profile: {
                            ...draft.profile,
                            sex: event.target.value as HevyDraft["profile"]["sex"],
                          },
                        })
                      }
                    >
                      <NativeSelectOption value="">Automatic</NativeSelectOption>
                      <NativeSelectOption value="female">Female</NativeSelectOption>
                      <NativeSelectOption value="male">Male</NativeSelectOption>
                    </NativeSelect>
                  </Field>
                </FieldGroup>
              </div>
            </div>
          </details>

        </div>
      )}
    </SettingsSection>
  );
}

function OptionalNumber({
  id,
  label,
  value,
  onChange,
}: {
  id: string;
  label: string;
  value: string;
  onChange: (value: string) => void;
}) {
  return (
    <Field>
      <FieldLabel htmlFor={id}>{label}</FieldLabel>
      <Input
        id={id}
        className="h-11"
        type="number"
        value={value}
        onChange={(event) => onChange(event.target.value)}
      />
    </Field>
  );
}
