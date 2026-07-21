import { SettingsSection } from "@/components/settings-shell";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Field,
  FieldContent,
  FieldDescription,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import {
  NativeSelect,
  NativeSelectOption,
} from "@/components/ui/native-select";
import type { SettingsState } from "@/lib/api";

type PreferencesDraft = SettingsState["preferences"];

/**
 * Presentational — the draft lives in the Settings route (settings.tsx),
 * which owns dirty-tracking and the Discard/Save footer shared across every
 * section. This component just renders the fields and reports edits up.
 */
export function PreferencesSettings({
  state,
  draft,
  onChange,
}: {
  state: SettingsState;
  draft: PreferencesDraft;
  onChange: (next: PreferencesDraft) => void;
}) {
  function update(patch: Partial<PreferencesDraft>) {
    onChange({ ...draft, ...patch });
  }

  return (
    <SettingsSection
      id="preferences"
      title="Preferences"
      description="Control polling, history, timezone, and hevy2garmin compatibility."
    >
      <div className="grid gap-6">
        <FieldGroup className="grid gap-5 md:grid-cols-2 lg:grid-cols-4">
          <NumberField
            id="garmin-poll"
            label="Garmin sync interval (min)"
            description="How often ActivSync checks Garmin. Recommended minimum: 10."
            value={draft.garminPollIntervalMinutes}
            onChange={(value) => update({ garminPollIntervalMinutes: value })}
          />
          <NumberField
            id="strava-poll"
            label="Strava sync interval (min)"
            description="How often ActivSync checks Strava. Recommended minimum: 2."
            value={draft.stravaPollIntervalMinutes}
            onChange={(value) => update({ stravaPollIntervalMinutes: value })}
          />
          <NumberField
            id="history-window"
            label="Activity history window (days)"
            description="How far back Garmin sync searches for activities."
            value={draft.lookbackDays}
            onChange={(value) => update({ lookbackDays: value })}
          />
          <Field>
            <FieldLabel htmlFor="display-timezone">Display timezone</FieldLabel>
            <NativeSelect
              id="display-timezone"
              className="w-full"
              value={draft.displayTimezone}
              onChange={(event) =>
                update({ displayTimezone: event.target.value })
              }
            >
              {state.timezones.map((timezone) => (
                <NativeSelectOption value={timezone} key={timezone}>
                  {timezone}
                </NativeSelectOption>
              ))}
            </NativeSelect>
          </Field>
        </FieldGroup>

        <div className="grid gap-4 rounded-xl border border-border/70 bg-muted/25 p-4">
          <Field orientation="horizontal">
            <Checkbox
              id="hevy-marker-enabled"
              checked={draft.hevy2garminMarkerEnabled}
              onCheckedChange={(checked) =>
                update({ hevy2garminMarkerEnabled: checked === true })
              }
            />
            <FieldContent>
              <FieldLabel htmlFor="hevy-marker-enabled">
                Auto-publish hevy2garmin imports held by category
              </FieldLabel>
              <FieldDescription>
                Enable only if a separate hevy2garmin app tags Garmin activity
                descriptions with the marker below.
              </FieldDescription>
            </FieldContent>
          </Field>
          <Field data-disabled={!draft.hevy2garminMarkerEnabled}>
            <FieldLabel htmlFor="hevy-marker">Marker text</FieldLabel>
            <Input
              id="hevy-marker"
              className="h-11"
              value={draft.hevy2garminMarker}
              disabled={!draft.hevy2garminMarkerEnabled}
              onChange={(event) =>
                update({ hevy2garminMarker: event.target.value })
              }
              required
            />
          </Field>
        </div>
      </div>
    </SettingsSection>
  );
}

function NumberField({
  id,
  label,
  description,
  value,
  onChange,
}: {
  id: string;
  label: string;
  description: string;
  value: number;
  onChange: (value: number) => void;
}) {
  return (
    <Field>
      <FieldLabel htmlFor={id}>{label}</FieldLabel>
      <Input
        id={id}
        className="h-11"
        type="number"
        min={1}
        value={value}
        onChange={(event) => onChange(event.target.valueAsNumber)}
        required
      />
      <FieldDescription>{description}</FieldDescription>
    </Field>
  );
}
