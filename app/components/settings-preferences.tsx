import { useState } from "react";

import { SettingsSection } from "@/components/settings-shell";
import { Button } from "@/components/ui/button";
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
import { Spinner } from "@/components/ui/spinner";
import { useSettingsAction } from "@/hooks/use-settings-action";
import { savePreferences, type SettingsState } from "@/lib/api";

export function PreferencesSettings({ state }: { state: SettingsState }) {
  const [preferences, setPreferences] = useState(state.preferences);
  const save = useSettingsAction(savePreferences);

  return (
    <SettingsSection
      id="preferences"
      title="Preferences"
      description="Control polling, history, timezone, and hevy2garmin compatibility."
    >
      <form
        className="grid gap-6"
        onSubmit={(event) => {
          event.preventDefault();
          save.mutate(preferences);
        }}
      >
        <FieldGroup className="grid gap-5 md:grid-cols-2">
          <NumberField
            id="garmin-poll"
            label="Garmin sync interval (min)"
            description="How often ActivSync checks Garmin. Recommended minimum: 10."
            value={preferences.garminPollIntervalMinutes}
            onChange={(value) =>
              setPreferences((current) => ({
                ...current,
                garminPollIntervalMinutes: value,
              }))
            }
          />
          <NumberField
            id="strava-poll"
            label="Strava sync interval (min)"
            description="How often ActivSync checks Strava. Recommended minimum: 2."
            value={preferences.stravaPollIntervalMinutes}
            onChange={(value) =>
              setPreferences((current) => ({
                ...current,
                stravaPollIntervalMinutes: value,
              }))
            }
          />
          <NumberField
            id="history-window"
            label="Activity history window (days)"
            description="How far back Garmin sync searches for activities."
            value={preferences.lookbackDays}
            onChange={(value) =>
              setPreferences((current) => ({
                ...current,
                lookbackDays: value,
              }))
            }
          />
          <Field>
            <FieldLabel htmlFor="display-timezone">Display timezone</FieldLabel>
            <NativeSelect
              id="display-timezone"
              className="w-full"
              value={preferences.displayTimezone}
              onChange={(event) =>
                setPreferences((current) => ({
                  ...current,
                  displayTimezone: event.target.value,
                }))
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

        <div className="grid gap-4 rounded-xl border bg-muted/25 p-4">
          <Field orientation="horizontal">
            <Checkbox
              id="hevy-marker-enabled"
              checked={preferences.hevy2garminMarkerEnabled}
              onCheckedChange={(checked) =>
                setPreferences((current) => ({
                  ...current,
                  hevy2garminMarkerEnabled: checked === true,
                }))
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
          <Field data-disabled={!preferences.hevy2garminMarkerEnabled}>
            <FieldLabel htmlFor="hevy-marker">Marker text</FieldLabel>
            <Input
              id="hevy-marker"
              className="h-11"
              value={preferences.hevy2garminMarker}
              disabled={!preferences.hevy2garminMarkerEnabled}
              onChange={(event) =>
                setPreferences((current) => ({
                  ...current,
                  hevy2garminMarker: event.target.value,
                }))
              }
              required
            />
          </Field>
        </div>

        <Button
          type="submit"
          className="h-11 sm:w-fit"
          disabled={save.isPending}
        >
          {save.isPending ? <Spinner /> : null}
          {save.isPending ? "Saving…" : "Save preferences"}
        </Button>
      </form>
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
