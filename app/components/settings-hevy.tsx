import { useState } from "react";

import { ConnectionStatus, SettingsSection } from "@/components/settings-shell";
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
import { HevyTools } from "@/components/settings-hevy-tools";
import {
  disconnectHevy,
  saveHevyCredentials,
  saveHevySettings,
  type SettingsState,
} from "@/lib/api";

export function HevySettings({ state }: { state: SettingsState }) {
  const [apiKey, setApiKey] = useState("");
  const [enabled, setEnabled] = useState(state.hevy.enabled);
  const [strategy, setStrategy] = useState(state.hevy.watchStrategy);
  const [graceMinutes, setGraceMinutes] = useState(state.hevy.graceMinutes);
  const [pollInterval, setPollInterval] = useState(state.hevy.pollIntervalMinutes);
  const [identity, setIdentity] = useState({
    manufacturer: nullableString(state.hevy.identity.manufacturer),
    product: nullableString(state.hevy.identity.product),
    serial: nullableString(state.hevy.identity.serial),
  });
  const [profile, setProfile] = useState({
    weightKg: nullableString(state.hevy.profileOverride.weightKg),
    birthYear: nullableString(state.hevy.profileOverride.birthYear),
    vo2max: nullableString(state.hevy.profileOverride.vo2max),
    sex: state.hevy.profileOverride.sex ?? "",
  });
  const connect = useSettingsAction(saveHevyCredentials);
  const save = useSettingsAction(saveHevySettings);
  const disconnect = useSettingsAction(disconnectHevy);

  return (
    <SettingsSection
      id="hevy"
      title="Hevy"
      description="Bring strength workouts from Hevy into Garmin, then publish the reviewed result to Strava."
    >
      <div className="grid gap-6">
        <ConnectionStatus
          name="Hevy"
          connected={state.hevy.connected}
          status={state.hevy.status}
        />
        {!state.hevy.connected ? (
          <form
            className="grid gap-4"
            onSubmit={(event) => {
              event.preventDefault();
              connect.mutate(apiKey);
            }}
          >
            <Field>
              <FieldLabel htmlFor="hevy-api-key">Hevy API key</FieldLabel>
              <Input
                id="hevy-api-key"
                className="h-11"
                type="password"
                value={apiKey}
                onChange={(event) => setApiKey(event.target.value)}
                required
              />
              <FieldDescription>
                Requires Hevy Pro. The key is validated before it is saved.
              </FieldDescription>
            </Field>
            <Button
              type="submit"
              className="h-11 sm:w-fit"
              disabled={connect.isPending}
            >
              {connect.isPending ? <Spinner /> : null}
              {connect.isPending ? "Validating…" : "Connect Hevy"}
            </Button>
          </form>
        ) : (
          <form
            className="grid gap-6"
            onSubmit={(event) => {
              event.preventDefault();
              save.mutate({
                enabled,
                watchStrategy: strategy,
                graceMinutes,
                pollIntervalMinutes: pollInterval,
                identity: {
                  manufacturer: parseNullableNumber(identity.manufacturer),
                  product: parseNullableNumber(identity.product),
                  serial: parseNullableNumber(identity.serial),
                },
                profileOverride: {
                  weightKg: parseNullableNumber(profile.weightKg),
                  birthYear: parseNullableNumber(profile.birthYear),
                  vo2max: parseNullableNumber(profile.vo2max),
                  sex: profile.sex === "male" || profile.sex === "female"
                    ? profile.sex
                    : null,
                },
              });
            }}
          >
            <Field orientation="horizontal">
              <Checkbox
                id="hevy-enabled"
                checked={enabled}
                onCheckedChange={(checked) => setEnabled(checked === true)}
              />
              <FieldContent>
                <FieldLabel htmlFor="hevy-enabled">
                  Enable Hevy sync
                </FieldLabel>
                <FieldDescription>
                  Poll Hevy for new workouts and reconcile them with Garmin.
                </FieldDescription>
              </FieldContent>
            </Field>
            <FieldGroup className="grid gap-5 md:grid-cols-3">
              <Field>
                <FieldLabel htmlFor="hevy-strategy">
                  Watch activity strategy
                </FieldLabel>
                <NativeSelect
                  id="hevy-strategy"
                  className="w-full"
                  value={strategy}
                  onChange={(event) =>
                    setStrategy(
                      event.target.value as SettingsState["hevy"]["watchStrategy"],
                    )
                  }
                >
                  <NativeSelectOption value="replace">Replace</NativeSelectOption>
                  <NativeSelectOption value="merge">Merge</NativeSelectOption>
                  <NativeSelectOption value="describe">Describe</NativeSelectOption>
                </NativeSelect>
              </Field>
              <Field>
                <FieldLabel htmlFor="hevy-grace">Grace period (min)</FieldLabel>
                <Input
                  id="hevy-grace"
                  className="h-11"
                  type="number"
                  min={0}
                  max={1440}
                  value={graceMinutes}
                  onChange={(event) => setGraceMinutes(event.target.valueAsNumber)}
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
                  value={pollInterval}
                  onChange={(event) => setPollInterval(event.target.valueAsNumber)}
                  required
                />
              </Field>
            </FieldGroup>
            <details className="rounded-xl border bg-muted/20 p-4">
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
                    {(["manufacturer", "product", "serial"] as const).map((key) => (
                      <Field key={key}>
                        <FieldLabel htmlFor={"identity-" + key}>
                          {capitalize(key)}
                        </FieldLabel>
                        <Input
                          id={"identity-" + key}
                          className="h-11"
                          type="number"
                          value={identity[key]}
                          onChange={(event) =>
                            setIdentity((current) => ({
                              ...current,
                              [key]: event.target.value,
                            }))
                          }
                        />
                      </Field>
                    ))}
                  </FieldGroup>
                </div>
                <div className="grid gap-2">
                  <p className="text-sm font-medium">Profile override</p>
                  <FieldGroup className="grid gap-4 sm:grid-cols-2">
                    <OptionalNumber
                      id="profile-weight"
                      label="Weight (kg)"
                      value={profile.weightKg}
                      onChange={(value) =>
                        setProfile((current) => ({ ...current, weightKg: value }))
                      }
                    />
                    <OptionalNumber
                      id="profile-birth"
                      label="Birth year"
                      value={profile.birthYear}
                      onChange={(value) =>
                        setProfile((current) => ({ ...current, birthYear: value }))
                      }
                    />
                    <OptionalNumber
                      id="profile-vo2"
                      label="VO₂ max"
                      value={profile.vo2max}
                      onChange={(value) =>
                        setProfile((current) => ({ ...current, vo2max: value }))
                      }
                    />
                    <Field>
                      <FieldLabel htmlFor="profile-sex">Sex</FieldLabel>
                      <NativeSelect
                        id="profile-sex"
                        className="w-full"
                        value={profile.sex}
                        onChange={(event) =>
                          setProfile((current) => ({
                            ...current,
                            sex: event.target.value,
                          }))
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
            <div className="flex flex-col gap-2 sm:flex-row">
              <Button
                type="submit"
                className="h-11"
                disabled={save.isPending || disconnect.isPending}
              >
                {save.isPending ? <Spinner /> : null}
                {save.isPending ? "Saving…" : "Save Hevy settings"}
              </Button>
              <Button
                type="button"
                variant="destructive"
                className="h-11"
                disabled={save.isPending || disconnect.isPending}
                onClick={() => {
                  if (window.confirm("Disconnect Hevy and pause Hevy sync?")) {
                    disconnect.mutate();
                  }
                }}
              >
                {disconnect.isPending ? <Spinner /> : null}
                {disconnect.isPending ? "Disconnecting…" : "Disconnect Hevy"}
              </Button>
            </div>
          </form>
        )}
        {state.hevy.connected ? <HevyTools /> : null}
      </div>
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

function nullableString(value: number | null) {
  return value === null ? "" : String(value);
}

function parseNullableNumber(value: string) {
  return value.trim() ? Number(value) : null;
}

function capitalize(value: string) {
  return value.charAt(0).toUpperCase() + value.slice(1);
}
