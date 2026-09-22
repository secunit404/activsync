import { useRef, useState, type FormEvent } from "react";

import { MfaCodeInput } from "@/components/mfa-code-input";
import { Button } from "@/components/ui/button";
import { ResponsiveOverlay } from "@/components/ui/responsive-overlay";
import {
  Field,
  FieldDescription,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import type { SettingsState } from "@/lib/api";

export type GarminStepProps = {
  state: SettingsState;
  pending: boolean;
  onConnect: (payload: {
    email: string;
    password: string;
    lookbackDays: number;
    detectedTimezone: string;
  }) => Promise<unknown>;
  onMfa: (code: string) => Promise<unknown>;
  onCancelMfa: () => Promise<unknown>;
};

/**
 * Step 1: Garmin sign-in (frame `2d`) plus the MFA prompt (frame `3c`),
 * layered on top as a `ResponsiveOverlay` rather than swapping the step's
 * own content — the overlay opens the instant `state.setup.mfaRequired`
 * comes back true and closes on verify, cancel, or any other dismissal.
 */
export function GarminStep({
  state,
  pending,
  onConnect,
  onMfa,
  onCancelMfa,
}: GarminStepProps) {
  const [email, setEmail] = useState(state.credentials.garminEmail);
  const [password, setPassword] = useState("");
  const [lookbackDays, setLookbackDays] = useState(
    state.preferences.lookbackDays,
  );
  const [code, setCode] = useState("");
  const mfaBodyRef = useRef<HTMLDivElement>(null);

  // Render-time reset (not a useEffect — this repo's ESLint enforces
  // react-hooks/set-state-in-effect): whenever the MFA overlay's open state
  // flips, in either direction, the code typed for the previous attempt is
  // stale and must not gate the next one's Verify button.
  const [appliedMfaOpen, setAppliedMfaOpen] = useState(state.setup.mfaRequired);
  if (appliedMfaOpen !== state.setup.mfaRequired) {
    setAppliedMfaOpen(state.setup.mfaRequired);
    setCode("");
  }

  function connectPayload() {
    return {
      email,
      password,
      lookbackDays,
      detectedTimezone: Intl.DateTimeFormat().resolvedOptions().timeZone ?? "",
    };
  }

  function focusFirstDigit(event: Event) {
    event.preventDefault();
    mfaBodyRef.current
      ?.querySelector<HTMLInputElement>('[aria-label="Digit 1"]')
      ?.focus();
  }

  const resendButton = (className: string) => (
    <Button
      type="button"
      variant="ghost"
      className={className}
      disabled={pending}
      onClick={() => void onConnect(connectPayload())}
    >
      Resend code
    </Button>
  );
  const cancelButton = (className: string) => (
    <Button
      type="button"
      variant="outline"
      className={className}
      disabled={pending}
      onClick={() => void onCancelMfa()}
    >
      Cancel
    </Button>
  );
  const verifyButton = (className: string) => (
    <Button
      type="button"
      className={className}
      disabled={pending || code.length < 6}
      onClick={() => void onMfa(code)}
    >
      {pending ? <Spinner /> : null}
      {pending ? "Verifying…" : "Verify"}
    </Button>
  );

  return (
    <div className="flex flex-1 flex-col gap-6 md:gap-7">
      <div className="flex flex-col gap-2">
        <p className="font-mono text-[11px] tracking-[0.2em] text-[var(--sync)] uppercase">
          Step 1 · Garmin
        </p>
        <h2 className="text-2xl font-extrabold tracking-[-0.02em] md:text-[30px]">
          Connect your Garmin account
        </h2>
        <p className="max-w-[52ch] text-sm leading-relaxed text-muted-foreground">
          ActivSync polls Garmin Connect and holds activities for review
          before they reach Strava. Your credentials are stored locally on
          your own server.
        </p>
      </div>
      <form
        className="grid max-w-[440px] gap-5"
        onSubmit={(event: FormEvent) => {
          event.preventDefault();
          void onConnect(connectPayload());
        }}
      >
        <FieldGroup>
          <Field>
            <FieldLabel htmlFor="setup-garmin-email">Email</FieldLabel>
            <Input
              id="setup-garmin-email"
              className="h-12"
              type="email"
              autoComplete="username"
              value={email}
              onChange={(event) => setEmail(event.target.value)}
              required
            />
          </Field>
          <Field>
            <FieldLabel htmlFor="setup-garmin-password">Password</FieldLabel>
            <Input
              id="setup-garmin-password"
              className="h-12"
              type="password"
              autoComplete="current-password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
              required={!state.credentials.garminPasswordSaved}
              placeholder={
                state.credentials.garminPasswordSaved
                  ? "Saved — leave blank to keep it"
                  : undefined
              }
            />
          </Field>
          <Field>
            <FieldLabel htmlFor="setup-lookback">
              Activity history window (days)
            </FieldLabel>
            <Input
              id="setup-lookback"
              className="h-12"
              type="number"
              min={1}
              value={lookbackDays}
              onChange={(event) => setLookbackDays(event.target.valueAsNumber)}
              required
            />
            <FieldDescription>
              How far back the first Garmin sync should search.
            </FieldDescription>
          </Field>
        </FieldGroup>
        <div className="flex items-center gap-2.5 rounded-[10px] border border-warning/22 bg-warning/7 px-3.5 py-2.5">
          <span
            className="size-1.5 shrink-0 rounded-full bg-warning"
            aria-hidden="true"
          />
          <p className="text-[12.5px] text-warning">
            If MFA is on, we’ll prompt for your code next.
          </p>
        </div>
        <Button type="submit" className="h-12 sm:w-fit" disabled={pending}>
          {pending ? <Spinner /> : null}
          {pending ? "Connecting…" : "Connect Garmin"}
        </Button>
      </form>
      <ResponsiveOverlay
        open={state.setup.mfaRequired}
        onOpenChange={(open) => {
          if (!open) {
            void onCancelMfa();
          }
        }}
        onOpenAutoFocus={focusFirstDigit}
        title="Enter your verification code"
        description="We sent a 6-digit code to your Garmin email. It expires in a few minutes."
        mobile="sheet"
        footer={
          <>
            <div className="flex items-center justify-between gap-2.5 md:hidden">
              {resendButton("")}
              {cancelButton("flex-1")}
            </div>
            {verifyButton("w-full md:hidden")}
            <div className="hidden items-center justify-between gap-2.5 md:flex">
              {resendButton("")}
              <div className="flex gap-2.5">
                {cancelButton("")}
                {verifyButton("")}
              </div>
            </div>
          </>
        }
      >
        <div ref={mfaBodyRef} className="flex flex-col gap-5">
          <p className="font-mono text-[10.5px] tracking-[0.14em] text-[var(--sync)] uppercase">
            Garmin · Two-factor
          </p>
          <MfaCodeInput length={6} onComplete={setCode} disabled={pending} />
        </div>
      </ResponsiveOverlay>
    </div>
  );
}
