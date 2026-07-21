import { Link } from "react-router";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";

/**
 * Frame `3a`'s "Backfill older workouts" card, as an entry point only — the
 * date picker, preview, and run steps live at `/hevy/backfill` (Task 16).
 */
export function HevyBackfillCard() {
  return (
    <Card aria-labelledby="hevy-backfill-title">
      <CardHeader>
        <CardTitle>
          <h2 id="hevy-backfill-title" className="text-[15px] font-bold">
            Backfill older workouts
          </h2>
        </CardTitle>
      </CardHeader>
      <CardContent className="flex flex-wrap items-center justify-between gap-4">
        <p className="max-w-md text-sm text-muted-foreground">
          Match historical Hevy workouts to Garmin activities from a chosen date.
        </p>
        <Button asChild className="shrink-0">
          <Link to="/hevy/backfill">Preview backfill →</Link>
        </Button>
      </CardContent>
    </Card>
  );
}
