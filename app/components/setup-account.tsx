import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useState, type FormEvent } from "react";

import { Button } from "@/components/ui/button";
import {
  Field,
  FieldDescription,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { PasswordInput } from "@/components/ui/password-input";
import { Spinner } from "@/components/ui/spinner";
import { claimOwner } from "@/lib/auth-api";
import { passwordsMismatch } from "@/lib/password";
import { queryKeys } from "@/lib/query-keys";

const MIN_PASSWORD = 15;

/**
 * Step 0: claim the install. The setup code is only in the server log, so
 * whoever finds a fresh public install first still cannot take it over.
 */
export function AccountStep() {
  const [setupCode, setSetupCode] = useState("");
  const [password, setPassword] = useState("");
  const [repeat, setRepeat] = useState("");
  const queryClient = useQueryClient();
  const claim = useMutation({
    mutationFn: claimOwner,
    onSuccess: async (status) => {
      queryClient.setQueryData(queryKeys.authStatus, status);
      await queryClient.invalidateQueries({ queryKey: queryKeys.settings });
    },
  });
  const mismatch = passwordsMismatch(password, repeat);
  const tooShort = password.length > 0 && password.length < MIN_PASSWORD;

  return (
    <div className="flex flex-1 flex-col gap-6 md:gap-7">
      <div className="flex flex-col gap-2">
        <p className="font-mono text-[11px] tracking-[0.2em] text-[var(--sync)] uppercase">
          Step 1: Account
        </p>
        <h2 className="text-2xl font-extrabold tracking-[-0.02em] md:text-[30px]">
          Claim this ActivSync
        </h2>
        <p className="max-w-[52ch] text-sm leading-relaxed text-muted-foreground">
          Enter the setup code from the server log and choose a password. With
          Docker, run <code className="font-mono">docker compose logs activsync</code> and
          look for “enter setup code”.
        </p>
      </div>
      <form
        className="grid max-w-[440px] gap-5"
        onSubmit={(event: FormEvent) => {
          event.preventDefault();
          claim.mutate({ setupCode, password });
        }}
      >
        <FieldGroup>
          <Field>
            <FieldLabel htmlFor="setup-code">Setup code</FieldLabel>
            <Input
              id="setup-code"
              className="h-12 font-mono uppercase"
              autoComplete="one-time-code"
              placeholder="XXXX-XXXX-XXXX-XXXX"
              value={setupCode}
              onChange={(event) => setSetupCode(event.target.value)}
              required
            />
          </Field>
          <Field data-invalid={tooShort || undefined}>
            <FieldLabel htmlFor="setup-password">Password</FieldLabel>
            <PasswordInput
              id="setup-password"
              className="h-12"
              autoComplete="new-password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
              required
            />
            <FieldDescription>
              At least {MIN_PASSWORD} characters. A few unrelated words make a
              strong, memorable passphrase.
            </FieldDescription>
          </Field>
          <Field data-invalid={mismatch || undefined}>
            <FieldLabel htmlFor="setup-password-repeat">Repeat password</FieldLabel>
            <PasswordInput
              id="setup-password-repeat"
              className="h-12"
              autoComplete="new-password"
              value={repeat}
              onChange={(event) => setRepeat(event.target.value)}
              required
            />
            {mismatch ? (
              <FieldDescription>The passwords don’t match.</FieldDescription>
            ) : null}
          </Field>
        </FieldGroup>
        {claim.isError ? (
          <p role="alert" className="text-sm text-destructive">
            {claim.error.message}
          </p>
        ) : null}
        <Button
          type="submit"
          className="h-12 sm:w-fit"
          disabled={claim.isPending || mismatch || password.length < MIN_PASSWORD}
        >
          {claim.isPending ? <Spinner /> : null}
          {claim.isPending ? "Claiming…" : "Claim ActivSync"}
        </Button>
      </form>
    </div>
  );
}
