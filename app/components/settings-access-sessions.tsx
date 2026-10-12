import { useQuery } from "@tanstack/react-query";

import { Button } from "@/components/ui/button";
import { useAuthAction } from "@/hooks/use-auth-action";
import {
  getSessions,
  revokeOtherSessions,
  revokeSession,
  type AuthSession,
} from "@/lib/auth-api";
import { queryKeys } from "@/lib/query-keys";

const METHOD_LABELS: Record<string, string> = {
  password: "Password",
  oidc: "Single sign-on",
  strava: "Strava",
};

function formatWhen(iso: string) {
  return new Date(iso).toLocaleString();
}

export function SessionList() {
  const sessions = useQuery({
    queryKey: queryKeys.authSessions,
    queryFn: ({ signal }) => getSessions(signal),
  });
  const revoke = useAuthAction(revokeSession);
  const revokeOthers = useAuthAction(revokeOtherSessions);
  const others = (sessions.data ?? []).filter((session) => !session.current);

  return (
    <div className="grid gap-3">
      <h3 className="text-sm font-semibold">Sessions</h3>
      <ul className="grid gap-2" aria-label="Signed-in sessions">
        {(sessions.data ?? []).map((session) => (
          <SessionRow
            key={session.publicId}
            session={session}
            pending={revoke.isPending}
            onRevoke={() => revoke.mutate(session.publicId)}
          />
        ))}
      </ul>
      <Button
        type="button"
        variant="outline"
        className="sm:w-fit"
        disabled={others.length === 0 || revokeOthers.isPending}
        onClick={() => revokeOthers.mutate()}
      >
        Sign out everywhere else
      </Button>
    </div>
  );
}

function SessionRow({
  session,
  pending,
  onRevoke,
}: {
  session: AuthSession;
  pending: boolean;
  onRevoke: () => void;
}) {
  return (
    <li className="flex items-center gap-3 rounded-lg border border-border/60 px-3 py-2 text-sm">
      <div className="min-w-0 flex-1">
        <p className="font-medium">
          {METHOD_LABELS[session.method] ?? session.method}
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
        <Button
          type="button"
          variant="ghost"
          size="sm"
          disabled={pending}
          onClick={onRevoke}
          aria-label={`Sign out the ${METHOD_LABELS[session.method] ?? session.method} session from ${session.ip || "an unknown address"}`}
        >
          Sign out
        </Button>
      )}
    </li>
  );
}
