import { ActivityIcon, CheckIcon } from "lucide-react";
import { Link, useSearchParams } from "react-router";

import { Button } from "@/components/ui/button";
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import type { StatusFilter } from "@/lib/api";

export function ActivitiesEmptyState({ status }: { status: StatusFilter | null }) {
  const [searchParams] = useSearchParams();

  if (status === "review") {
    const all = new URLSearchParams(searchParams);
    all.set("status", "all");
    all.delete("page");
    return (
      <Empty className="min-h-64 border-solid bg-card">
        <EmptyHeader>
          <EmptyMedia variant="icon" className="text-success">
            <CheckIcon />
          </EmptyMedia>
          <EmptyTitle className="text-base">Nothing waiting for you</EmptyTitle>
          <EmptyDescription>
            No activity is held for review or missing from Strava.
          </EmptyDescription>
        </EmptyHeader>
        <EmptyContent>
          <Button asChild variant="outline" size="xl">
            <Link to={{ search: all.toString() }}>Show all activities</Link>
          </Button>
        </EmptyContent>
      </Empty>
    );
  }

  return (
    <Empty className="min-h-64">
      <EmptyHeader>
        <EmptyMedia variant="icon">
          <ActivityIcon />
        </EmptyMedia>
        <EmptyTitle>No activities found</EmptyTitle>
        <EmptyDescription>
          {status
            ? `There are no ${status} activities in the current sync history.`
            : "Activities will appear here after the first Garmin sync."}
        </EmptyDescription>
      </EmptyHeader>
    </Empty>
  );
}
