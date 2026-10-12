import { useMutation, useQueryClient } from "@tanstack/react-query";
import { CircleAlertIcon } from "lucide-react";
import { useEffect, useState, type FormEvent, type ReactNode } from "react";
import { Navigate, useNavigate, useSearchParams } from "react-router";

import { AppBrand } from "@/components/app-shell";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Field, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { useAuthStatus } from "@/hooks/use-auth-status";
import { LOGIN_ERRORS } from "@/lib/auth-errors";
import {
  login,
  oidcLoginUrl,
  stravaLoginUrl,
  type AuthStatus,
} from "@/lib/auth-api";
import { queryKeys } from "@/lib/query-keys";
import type { Route } from "./+types/login";

export function meta(): Route.MetaDescriptors {
  return [{ title: "Sign in - ActivSync" }];
}

// Judged on the parsed URL, not the raw string: the URL parser strips tabs
// and newlines, reads `\` as `/`, and collapses dot segments, so
// "/\t/evil.test" and "/.//evil.test" both come out as "//evil.test". The
// normalized path is re-checked because it can itself be protocol-relative.
export function safeNext(value: string | null): string {
  if (!value || !value.startsWith("/")) return "/";
  const origin = window.location.origin;
  try {
    const url = new URL(value, origin);
    if (url.origin !== origin || url.pathname.startsWith("//")) return "/";
    return `${url.pathname}${url.search}${url.hash}`;
  } catch {
    return "/";
  }
}

export default function Login() {
  const [searchParams] = useSearchParams();
  const status = useAuthStatus();
  const next = safeNext(searchParams.get("next"));
  const local = searchParams.get("local") === "1";

  if (status.isPending) {
    return (
      <LoginCard>
        <div className="flex items-center gap-2 text-muted-foreground">
          <Spinner /> Loading…
        </div>
      </LoginCard>
    );
  }
  if (status.isError) {
    return (
      <LoginCard>
        <Alert variant="destructive">
          <CircleAlertIcon />
          <AlertTitle>Couldn’t reach ActivSync</AlertTitle>
          <AlertDescription>{status.error.message}</AlertDescription>
        </Alert>
      </LoginCard>
    );
  }
  if (!status.data.ownerClaimed) {
    return <Navigate to="/setup" replace />;
  }
  if (status.data.authenticated) {
    return <Navigate to={next} replace />;
  }
  return (
    <LoginView
      status={status.data}
      next={next}
      local={local}
      error={searchParams.get("error")}
    />
  );
}

export function LoginView({
  status,
  next,
  local,
  error,
}: {
  status: AuthStatus;
  next: string;
  local: boolean;
  error: string | null;
}) {
  const oidc = status.login.oidc;
  const autoRedirect = Boolean(oidc?.autoRedirect && !local && !error);

  useEffect(() => {
    if (autoRedirect) window.location.assign(oidcLoginUrl(next));
  }, [autoRedirect, next]);

  if (autoRedirect) {
    return (
      <LoginCard>
        <div className="flex items-center gap-2 text-muted-foreground">
          <Spinner /> Redirecting to {oidc?.label}…
        </div>
      </LoginCard>
    );
  }

  return (
    <LoginCard>
      <div className="grid gap-1.5">
        <h1 className="text-2xl font-extrabold tracking-[-0.02em]">Sign in</h1>
        <p className="text-sm text-muted-foreground">
          Sign in to manage your activities.
        </p>
      </div>
      {error ? (
        <Alert variant="destructive">
          <CircleAlertIcon />
          <AlertTitle>Sign-in failed</AlertTitle>
          <AlertDescription>
            {LOGIN_ERRORS[error] ?? "Sign-in failed. Try again."}
          </AlertDescription>
        </Alert>
      ) : null}
      {oidc || status.login.strava ? (
        <div className="grid gap-2.5">
          {oidc ? (
            <Button asChild size="lg" className="h-11">
              <a href={oidcLoginUrl(next)}>Sign in with {oidc.label}</a>
            </Button>
          ) : null}
          {status.login.strava ? (
            <Button asChild size="lg" variant="outline" className="h-11">
              <a href={stravaLoginUrl(next)}>Sign in with Strava</a>
            </Button>
          ) : null}
        </div>
      ) : null}
      <PasswordForm next={next} />
    </LoginCard>
  );
}

function PasswordForm({ next }: { next: string }) {
  const [password, setPassword] = useState("");
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const mutation = useMutation({
    mutationFn: login,
    onSuccess: (status) => {
      queryClient.setQueryData(queryKeys.authStatus, status);
      navigate(next, { replace: true });
    },
  });

  return (
    <form
      className="grid gap-4"
      onSubmit={(event: FormEvent) => {
        event.preventDefault();
        mutation.mutate(password);
      }}
    >
      <FieldGroup>
        <Field>
          <FieldLabel htmlFor="login-password">Password</FieldLabel>
          <Input
            id="login-password"
            className="h-11"
            type="password"
            autoComplete="current-password"
            autoFocus
            value={password}
            onChange={(event) => setPassword(event.target.value)}
            required
          />
        </Field>
      </FieldGroup>
      {mutation.isError ? (
        <p role="alert" className="text-sm text-destructive">
          {mutation.error.message}
        </p>
      ) : null}
      <Button type="submit" size="lg" className="h-11" disabled={mutation.isPending}>
        {mutation.isPending ? <Spinner /> : null}
        {mutation.isPending ? "Signing in…" : "Sign in"}
      </Button>
    </form>
  );
}

function LoginCard({ children }: { children: ReactNode }) {
  return (
    <div className="grid min-h-screen place-items-center bg-background px-4 py-10">
      <main className="flex w-full max-w-[400px] flex-col gap-6 rounded-2xl border border-border bg-card p-7 text-card-foreground shadow-[var(--shadow-frame)]">
        <AppBrand />
        {children}
      </main>
    </div>
  );
}
