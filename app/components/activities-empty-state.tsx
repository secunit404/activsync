import { ActivityIcon, CheckIcon } from "lucide-react";
import { Link, useSearchParams, type To } from "react-router";

import { Button } from "@/components/ui/button";
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import { reviewCountOf } from "@/lib/activity-view";
import type { PollIntervals, PublishStatus, StatusFilter } from "@/lib/api";

type NextStep = { label: string; to: To };
type EmptyCopy = { title: string; description: string; next?: NextStep };

export function ActivitiesEmptyState({
  status,
  counts,
  pollIntervals,
}: {
  status: StatusFilter | null;
  counts?: Record<PublishStatus, number>;
  pollIntervals?: PollIntervals;
}) {
  const [searchParams] = useSearchParams();
  const viewSearch = (view: string) => {
    const next = new URLSearchParams(searchParams);
    next.set("status", view);
    next.delete("page");
    return { search: next.toString() };
  };
  const showAll = { label: "Show all activities", to: viewSearch("all") };

  if (status === "review") {
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
        <EmptyNextStep next={showAll} />
      </Empty>
    );
  }

  const copy = emptyCopy(status, {
    garminMinutes: pollIntervals?.garminMinutes,
    reviewCount: counts ? reviewCountOf(counts) : 0,
    showAll,
    review: { label: "Show what needs review", to: viewSearch("review") },
  });

  return (
    <Empty className="min-h-64">
      <EmptyHeader>
        <EmptyMedia variant="icon">
          <ActivityIcon />
        </EmptyMedia>
        <EmptyTitle className="text-base">{copy.title}</EmptyTitle>
        <EmptyDescription>{copy.description}</EmptyDescription>
      </EmptyHeader>
      {copy.next ? <EmptyNextStep next={copy.next} /> : null}
    </Empty>
  );
}

function EmptyNextStep({ next }: { next: NextStep }) {
  return (
    <EmptyContent>
      <Button asChild variant="outline" size="xl">
        <Link to={next.to}>{next.label}</Link>
      </Button>
    </EmptyContent>
  );
}

function emptyCopy(
  status: Exclude<StatusFilter, "review"> | null,
  context: {
    garminMinutes?: number;
    reviewCount: number;
    showAll: NextStep;
    review: NextStep;
  },
): EmptyCopy {
  switch (status) {
    case "pending":
      return {
        title: "Nothing waiting to publish",
        description:
          "New activities whose type has auto-sync on wait here until the next Strava check.",
        next: { label: "Choose which types auto-sync", to: "/settings#autosync" },
      };
    case "held":
      return {
        title: "Nothing held for review",
        description: "Activities whose type has auto-sync off wait here for you.",
        next: context.showAll,
      };
    case "missing":
      return {
        title: "Nothing missing from Strava",
        description: "A published activity that is removed from Strava shows up here.",
        next: context.showAll,
      };
    case "published":
      return {
        title: "Nothing published yet",
        description: "Activities show up here once they're on Strava.",
        next: context.reviewCount > 0 ? context.review : undefined,
      };
    case "excluded":
      return {
        title: "Nothing excluded",
        description:
          "Exclude an activity to keep it off Strava for good. You can restore it later.",
        next: context.reviewCount > 0 ? context.review : undefined,
      };
    case null:
      return {
        title: "No activities yet",
        description: context.garminMinutes
          ? `ActivSync checks Garmin every ${context.garminMinutes} min. New activities show up here after the next check.`
          : "New activities show up here after the next Garmin check.",
        next: { label: "Check connections", to: "/settings#connections" },
      };
  }
}
