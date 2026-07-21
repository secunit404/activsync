import { CircleAlertIcon } from "lucide-react";

import { Button } from "@/components/ui/button";

type AttentionBannerProps = {
  service: "garmin" | "strava";
  onReconnect: () => void;
};

const copy: Record<AttentionBannerProps["service"], { title: string; description: string }> = {
  strava: {
    title: "Reconnect Strava to resume publishing",
    description:
      "Activities keep syncing from Garmin, but publishing is paused until Strava is reconnected.",
  },
  garmin: {
    title: "Reconnect Garmin to resume syncing",
    description:
      "Strava publishing continues for activities already synced, but new Garmin activities won't appear until Garmin is reconnected.",
  },
};

/**
 * "Sync needs attention" banner (handoff frame `3b`). `AppState.connections
 * .broken` can name both services at once — the caller (activities.tsx)
 * renders one `AttentionBanner` per entry rather than this component trying
 * to describe two outages in a single sentence.
 */
export function AttentionBanner({ service, onReconnect }: AttentionBannerProps) {
  const { title, description } = copy[service];

  return (
    <div
      role="alert"
      className="flex items-start gap-3 rounded-xl border border-destructive/30 bg-destructive/[0.07] p-4"
    >
      <span
        aria-hidden="true"
        className="grid size-[22px] shrink-0 place-items-center rounded-full bg-destructive/15 text-destructive"
      >
        <CircleAlertIcon className="size-3.5" />
      </span>
      <div className="grid flex-1 gap-0.5">
        <p className="text-sm font-bold text-destructive">{title}</p>
        <p className="text-[13px] leading-relaxed text-destructive/80">{description}</p>
      </div>
      <Button variant="destructive" className="h-9 shrink-0" onClick={onReconnect}>
        Reconnect
      </Button>
    </div>
  );
}
