import { useState } from "react";

import { Button } from "@/components/ui/button";
import { Field, FieldDescription, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";

export type HevyStepProps = {
  pending: boolean;
  onConnect: (apiKey: string) => Promise<unknown>;
  onSkip: () => Promise<unknown>;
};

/** Step 3: Hevy is optional — connect it now or skip and add it later. */
export function HevyStep({ pending, onConnect, onSkip }: HevyStepProps) {
  const [apiKey, setApiKey] = useState("");
  return (
    <div className="flex flex-1 flex-col gap-6 md:gap-7">
      <div className="flex flex-col gap-2">
        <p className="font-mono text-[11px] tracking-[0.2em] text-[var(--sync)] uppercase">
          Step 3 · Hevy
        </p>
        <h2 className="text-2xl font-extrabold tracking-[-0.02em] md:text-[30px]">
          Connect Hevy (optional)
        </h2>
        <p className="max-w-[52ch] text-sm leading-relaxed text-muted-foreground">
          Sync Hevy strength workouts onto Garmin and onward to Strava. Hevy
          Pro is required.
        </p>
      </div>
      <form
        className="grid max-w-[440px] gap-5"
        onSubmit={(event) => {
          event.preventDefault();
          void onConnect(apiKey);
        }}
      >
        <Field>
          <FieldLabel htmlFor="setup-hevy-key">Hevy API key</FieldLabel>
          <Input
            id="setup-hevy-key"
            className="h-12"
            type="password"
            value={apiKey}
            onChange={(event) => setApiKey(event.target.value)}
            required
          />
          <FieldDescription>
            Disable any standalone hevy2garmin instance before enabling this.
          </FieldDescription>
        </Field>
        <div className="flex flex-col gap-2.5 sm:flex-row">
          <Button type="submit" className="h-12 sm:w-fit" disabled={pending}>
            {pending ? <Spinner /> : null}
            {pending ? "Validating…" : "Connect Hevy"}
          </Button>
          <Button
            type="button"
            variant="outline"
            className="h-12 sm:w-fit"
            disabled={pending}
            onClick={() => void onSkip()}
          >
            Skip for now
          </Button>
        </div>
      </form>
    </div>
  );
}
