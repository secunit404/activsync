import { CircleAlertIcon } from "lucide-react";

import { Button } from "@/components/ui/button";

type ConnectionErrorProps = {
  onRetry: () => void;
  /**
   * HTTP status of the failed request, when the failure was an `ApiError`
   * (the server responded, just with an error) rather than the request
   * never reaching it at all (network failure, container down). Presence of
   * a status is what distinguishes the two — see `activities.tsx`, which is
   * the only caller and routes both kinds of failure here.
   */
  status?: number;
  /** The backend's `detail` message for an `ApiError`, shown as a secondary
   * line so a 500 (say) is diagnosable instead of just "couldn't reach". */
  detail?: string;
};

/**
 * Full-block connection-failure state (handoff frame `3b`). The Activities
 * route renders this in place of the page when the initial `appState`/
 * `activities` requests fail outright — e.g. the backend isn't reachable —
 * as opposed to a query returning a normal error payload for one activity.
 * Layout-agnostic (no page padding baked in) so the caller controls
 * placement; see `activities.tsx`.
 */
export function ConnectionError({ onRetry, status, detail }: ConnectionErrorProps) {
  return (
    <div role="alert" className="grid max-w-sm gap-3 rounded-xl border border-destructive/30 p-6">
      <span
        aria-hidden="true"
        className="grid size-9 place-items-center rounded-[10px] bg-destructive/[0.12] text-destructive"
      >
        <CircleAlertIcon className="size-[18px]" />
      </span>
      <div className="grid gap-1">
        <p className="text-[15px] font-bold">Couldn’t reach ActivSync</p>
        <p className="text-[12.5px] leading-relaxed text-muted-foreground">
          {status
            ? `The server responded with an error (${status}).`
            : "The server didn’t respond. Check that the container is running."}
        </p>
        {detail ? (
          <p className="text-[12.5px] leading-relaxed text-muted-foreground/80">{detail}</p>
        ) : null}
      </div>
      <Button className="h-10 w-fit" onClick={onRetry}>
        Try again
      </Button>
    </div>
  );
}
