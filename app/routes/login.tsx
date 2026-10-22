import { useMutation, useQueryClient } from "@tanstack/react-query";
import { CircleAlertIcon, KeyRoundIcon } from "lucide-react";
import { useEffect, useState, type FormEvent, type ReactNode } from "react";
import { Navigate, useNavigate, useSearchParams } from "react-router";

import { AppBrand } from "@/components/app-shell";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Field, FieldGroup, FieldLabel } from "@/components/ui/field";
import { PasswordInput } from "@/components/ui/password-input";
import { Spinner } from "@/components/ui/spinner";
import { useAuthStatus } from "@/hooks/use-auth-status";
import { LOGIN_ERRORS } from "@/lib/auth-errors";
import { login, oidcLoginUrl, type AuthStatus } from "@/lib/auth-api";
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
    <LoginCard subtitle="Sign in to manage your activities">
      {error ? (
        <Alert variant="destructive">
          <CircleAlertIcon />
          <AlertTitle>Sign-in failed</AlertTitle>
          <AlertDescription>
            {LOGIN_ERRORS[error] ?? "Sign-in failed. Try again."}
          </AlertDescription>
        </Alert>
      ) : null}
      <PasswordForm next={next} />
      {oidc ? (
        <>
          <OrDivider />
          <ProviderButton
            href={oidcLoginUrl(next)}
            icon={<KeyRoundIcon className="size-5" aria-hidden="true" />}
          >
            Sign in with {oidc.label}
          </ProviderButton>
        </>
      ) : null}
    </LoginCard>
  );
}

function OrDivider() {
  return (
    <div className="flex items-center gap-3 text-xs text-muted-foreground">
      <span className="h-px flex-1 bg-border" />
      or
      <span className="h-px flex-1 bg-border" />
    </div>
  );
}

function ProviderButton({
  href,
  icon,
  children,
}: {
  href: string;
  icon: ReactNode;
  children: ReactNode;
}) {
  return (
    <a
      href={href}
      className="inline-flex h-11 w-full items-center justify-center gap-2.5 rounded-lg border border-input bg-muted px-4 text-sm font-medium text-foreground transition-colors outline-none hover:bg-input focus-visible:ring-3 focus-visible:ring-ring/50"
    >
      {icon}
      {children}
    </a>
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
          <PasswordInput
            id="login-password"
            className="h-11"
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

// The version lives in the signed-in app state, which this page can't read,
// so the footer carries only the repository link.
const REPO_URL = "https://github.com/secunit404/activsync";

function LoginCard({ subtitle, children }: { subtitle?: string; children: ReactNode }) {
  return (
    <div className="flex min-h-screen flex-col items-center justify-center gap-4 bg-background px-4 py-10">
      <main className="flex w-full max-w-[400px] flex-col gap-6 rounded-2xl border border-border bg-card p-7 text-card-foreground shadow-[var(--shadow-frame)]">
        <div className="grid gap-1.5 text-center">
          <h1>
            <AppBrand className="text-[26px]" />
          </h1>
          {subtitle ? <p className="text-sm text-muted-foreground">{subtitle}</p> : null}
        </div>
        {children}
      </main>
      <footer className="text-xs text-muted-foreground">
        <a
          className="font-normal text-muted-foreground hover:text-foreground"
          href={REPO_URL}
          target="_blank"
          rel="noreferrer"
        >
          GitHub
        </a>
      </footer>
    </div>
  );
}
