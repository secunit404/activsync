import { Link } from "react-router";

import { cn } from "@/lib/utils";

/**
 * Says why Publish is disabled while Strava is disconnected, in the open: a `title` never shows on touch,
 * and an owner checking in from a phone needs the way out next to the
 * button. Publish points at this line with `aria-describedby`.
 */
export function PublishBlockedNote({
  id,
  className,
}: {
  id: string;
  className?: string;
}) {
  return (
    <p id={id} className={cn("text-[13px] leading-snug text-muted-foreground", className)}>
      <Link
        to="/settings#connections"
        className="rounded-sm font-semibold text-foreground underline underline-offset-3 outline-none hover:text-primary focus-visible:ring-3 focus-visible:ring-ring"
      >
        Reconnect Strava
      </Link>{" "}
      to publish.
    </p>
  );
}
