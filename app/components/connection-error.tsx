import { CircleAlertIcon } from "lucide-react";

import { Button } from "@/components/ui/button";

type ConnectionErrorProps = {
  onRetry: () => void;
};

/**
 * Full-block connection-failure state (handoff frame `3b`). The Activities
 * route renders this in place of the page when the initial `appState`/
 * `activities` requests fail outright — e.g. the backend isn't reachable —
 * as opposed to a query returning a normal error payload for one activity.
 * Layout-agnostic (no page padding baked in) so the caller controls
 * placement; see `activities.tsx`.
 */
export function ConnectionError({ onRetry }: ConnectionErrorProps) {
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
          The server didn’t respond. Check that the container is running.
        </p>
      </div>
      <Button className="h-10 w-fit" onClick={onRetry}>
        Try again
      </Button>
    </div>
  );
}
