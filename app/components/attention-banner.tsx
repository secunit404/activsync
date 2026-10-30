import { CircleAlertIcon } from "lucide-react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

type Service = "garmin" | "strava";

type AttentionBannerProps = {
  /** Every broken service; both at once read as one outage, not two banners. */
  services: Service[];
  onReconnect: (service: Service) => void;
};

const single: Record<Service, { title: string; description: string }> = {
  strava: {
    title: "Reconnect Strava to resume publishing",
    description:
      "Garmin activities keep syncing. Nothing publishes until you reconnect.",
  },
  garmin: {
    title: "Reconnect Garmin to resume syncing",
    description:
      "Synced activities still publish. New ones appear once you reconnect.",
  },
};

const both = {
  title: "Reconnect Garmin and Strava",
  description: "Nothing new syncs from Garmin, and nothing publishes to Strava, until you reconnect.",
};

const serviceName: Record<Service, string> = { garmin: "Garmin", strava: "Strava" };

/**
 * "Sync needs attention" banner (handoff frame `3b`). One banner however
 * many services are broken: two stacked red cards took a phone's first
 * screen. `role="status"`, not `alert`: an outage is standing state that is
 * already there on load, not a new error to interrupt with.
 *
 * The body is full-strength `text-destructive` (6.0:1 on the 7% tint); the
 * 80% tint it replaced measured 4.3:1.
 */
export function AttentionBanner({ services, onReconnect }: AttentionBannerProps) {
  if (services.length === 0) {
    return null;
  }
  const pair = services.length > 1;
  const { title, description } = pair ? both : single[services[0]];

  return (
    <div
      role="status"
      // On a phone the description runs under the button too: beside it,
      // each line was squeezed to half the card and the banner doubled in
      // height. Two buttons get their own row there for the same reason.
      className="grid grid-cols-[auto_minmax(0,1fr)_auto] items-start gap-x-3 gap-y-0.5 rounded-xl border border-destructive/30 bg-destructive/[0.07] p-4"
    >
      <span
        aria-hidden="true"
        className="row-span-2 grid size-[22px] place-items-center rounded-full bg-destructive/15 text-destructive"
      >
        <CircleAlertIcon className="size-3.5" />
      </span>
      <p className="text-sm font-bold text-destructive">{title}</p>
      <div
        className={cn(
          "flex flex-wrap gap-2 md:row-span-2 md:self-center",
          pair && "max-md:col-span-2 max-md:col-start-2 max-md:row-start-3 max-md:mt-2.5",
        )}
      >
        {services.map((service) => (
          <Button
            key={service}
            variant="destructive"
            className="h-9"
            onClick={() => onReconnect(service)}
          >
            {pair ? `Reconnect ${serviceName[service]}` : "Reconnect"}
          </Button>
        ))}
      </div>
      <p className="text-[13px] leading-relaxed text-destructive max-md:col-span-2">
        {description}
      </p>
    </div>
  );
}
