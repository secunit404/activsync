import { useQuery } from "@tanstack/react-query";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { useAuthAction } from "@/hooks/use-auth-action";
import {
  getSessions,
  revokeOtherSessions,
  revokeSession,
  type AuthSession,
} from "@/lib/auth-api";
import { queryKeys } from "@/lib/query-keys";
import { describeUserAgent } from "@/lib/user-agent";

const METHOD_LABELS: Record<string, string> = {
  password: "Password",
  oidc: "Single sign-on",
};

function formatWhen(iso: string) {
  return new Date(iso).toLocaleString();
}

export function SessionList() {
  const sessions = useQuery({
    queryKey: queryKeys.authSessions,
    queryFn: ({ signal }) => getSessions(signal),
  });
  const revokeOthers = useAuthAction(revokeOtherSessions);
  const [confirmingOthers, setConfirmingOthers] = useState(false);
  const others = (sessions.data ?? []).filter((session) => !session.current);

  return (
    <div className="grid min-w-0 gap-3">
      <h3 className="text-sm font-semibold">Sessions</h3>
      <ul className="grid min-w-0 gap-2" aria-label="Signed-in sessions">
        {(sessions.data ?? []).map((session) => (
          <SessionRow key={session.publicId} session={session} />
        ))}
      </ul>
      <Button
        type="button"
        variant="outline"
        className="sm:w-fit"
        disabled={others.length === 0 || revokeOthers.isPending}
        onClick={() => setConfirmingOthers(true)}
      >
        Sign out everywhere else
      </Button>
      <ConfirmDialog
        open={confirmingOthers}
        onOpenChange={setConfirmingOthers}
        title="Sign out everywhere else?"
        description="Every other browser and device will be signed out. Only this browser stays signed in."
        confirmLabel="Sign out everywhere else"
        pendingLabel="Signing out…"
        pending={revokeOthers.isPending}
        onConfirm={() =>
          revokeOthers.mutate(undefined, { onSuccess: () => setConfirmingOthers(false) })
        }
      />
    </div>
  );
}

function SessionRow({ session }: { session: AuthSession }) {
  const revoke = useAuthAction(revokeSession);
  const [confirming, setConfirming] = useState(false);
  const methodLabel = METHOD_LABELS[session.method] ?? session.method;
  const device = describeUserAgent(session.userAgent);
  const name = device ?? `the ${methodLabel} session from ${session.ip || "an unknown address"}`;

  return (
    <li className="flex min-w-0 items-center gap-3 rounded-lg border border-border/60 px-3 py-2 text-sm">
      <div className="min-w-0 flex-1">
        <p className="truncate font-medium">
          {device ? `${methodLabel} · ${device}` : methodLabel}
          {session.current ? (
            <span className="ml-2 text-xs text-[var(--sync)]">This browser</span>
          ) : null}
        </p>
        <p className="truncate font-mono text-xs text-muted-foreground" title={session.userAgent}>
          {session.ip || "unknown address"} · last seen {formatWhen(session.lastSeenAt)} · since{" "}
          {formatWhen(session.createdAt)}
        </p>
      </div>
      {session.current ? null : (
        <>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            disabled={revoke.isPending}
            onClick={() => setConfirming(true)}
            aria-label={`Sign out ${name}`}
          >
            Sign out
          </Button>
          <ConfirmDialog
            open={confirming}
            onOpenChange={setConfirming}
            title="Sign out this browser?"
            description={`${device ?? `The ${methodLabel} session from ${session.ip || "an unknown address"}`} will be signed out. It will need to sign in again to get back in.`}
            confirmLabel="Sign out"
            pendingLabel="Signing out…"
            pending={revoke.isPending}
            onConfirm={() =>
              revoke.mutate(session.publicId, { onSuccess: () => setConfirming(false) })
            }
          />
        </>
      )}
    </li>
  );
}
