import { useEffect, useState, type FormEvent } from "react";
import {
  CheckCircle2Icon,
  ExternalLinkIcon,
  TriangleAlertIcon,
} from "lucide-react";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import {
  Field,
  FieldError,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { Textarea } from "@/components/ui/textarea";
import type {
  ActivityActionInput,
  ActivityActionState,
} from "@/hooks/use-activity-actions";
import type { Activity, ActivityActionResult, PublishStatus } from "@/lib/api";

const statusVariants: Record<
  PublishStatus,
  "default" | "secondary" | "destructive" | "outline"
> = {
  pending: "default",
  held: "outline",
  published: "secondary",
  missing: "destructive",
  excluded: "outline",
};

const statusLabels: Record<PublishStatus, string> = {
  pending: "Pending",
  held: "Held",
  published: "Published",
  missing: "Missing",
  excluded: "Excluded",
};

type ActivityCardProps = {
  activity: Activity;
  connectionsBroken: boolean;
  selectionMode: boolean;
  selected: boolean;
  actionState: ActivityActionState;
  onSelectedChange: (selected: boolean) => void;
  onAction: (action: ActivityActionInput) => Promise<ActivityActionResult>;
};

type EditFeedback = { kind: "success" | "error"; message: string } | null;

export function ActivityCard({
  activity,
  connectionsBroken,
  selectionMode,
  selected,
  actionState,
  onSelectedChange,
  onAction,
}: ActivityCardProps) {
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editing, setEditing] = useState(false);
  const [title, setTitle] = useState(activity.title);
  const [description, setDescription] = useState(activity.description);
  const [editFeedback, setEditFeedback] = useState<EditFeedback>(null);
  const publishable = isPublishable(activity.publishStatus);
  const publishing = isPendingAction(actionState, "publish", activity.garminActivityId);
  const excluding = isPendingAction(actionState, "exclude", activity.garminActivityId);
  const restoring = isPendingAction(actionState, "restore", activity.garminActivityId);
  const saving = isPendingAction(actionState, "edit", activity.garminActivityId);
  const controlsDisabled = actionState.isPending;
  const metrics = activityMetrics(activity);
  const summaryMetrics = activitySummaryMetrics(activity);

  useEffect(() => {
    if (editFeedback?.kind !== "success") {
      return;
    }
    const timeout = window.setTimeout(() => setEditFeedback(null), 2500);
    return () => window.clearTimeout(timeout);
  }, [editFeedback]);

  const setOpen = (open: boolean) => {
    setDialogOpen(open);
    if (open) {
      setTitle(activity.title);
      setDescription(activity.description);
    } else {
      setEditing(false);
      setEditFeedback(null);
    }
  };

  const saveActivity = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!title.trim()) {
      setEditFeedback({ kind: "error", message: "Activity title cannot be blank" });
      return;
    }
    setEditFeedback(null);
    try {
      const result = await onAction({
        type: "edit",
        activityId: activity.garminActivityId,
        title,
        description,
      });
      setEditing(false);
      setEditFeedback({ kind: "success", message: result.message });
    } catch (error) {
      setEditFeedback({
        kind: "error",
        message: error instanceof Error ? error.message : "Could not save activity",
      });
    }
  };

  const runAction = (action: ActivityActionInput) => {
    void onAction(action).catch(() => undefined);
  };

  return (
    <Card
      size="sm"
      className="[contain-intrinsic-size:auto_210px] [content-visibility:auto]"
    >
      <CardHeader className="grid-cols-[1fr_auto] gap-x-3">
        <div className="grid min-w-0 gap-1">
          <CardDescription className="flex flex-wrap items-center gap-2">
            <span>
              {activity.startDateDisplay} · {activity.startClockDisplay}
            </span>
            <span className="capitalize">
              {formatActivityType(activity.activityType)}
            </span>
          </CardDescription>
          <CardTitle>
            <h3 className="truncate text-base font-semibold">{activity.title}</h3>
          </CardTitle>
        </div>
        <CardAction className="flex flex-col items-end gap-1.5">
          <Badge variant={statusVariants[activity.publishStatus]}>
            {statusLabels[activity.publishStatus]}
          </Badge>
          {activity.hevyBadge ? (
            <Badge variant="outline">Hevy · {activity.hevyBadge}</Badge>
          ) : null}
        </CardAction>
      </CardHeader>

      <CardContent className="grid gap-3">
        {summaryMetrics.length > 0 ? (
          <MetricGrid metrics={summaryMetrics.slice(0, 4)} />
        ) : null}

        {activity.description ? (
          <p className="line-clamp-2 text-sm leading-6 text-muted-foreground">
            {activity.description}
          </p>
        ) : null}

        <ActivityLinks activity={activity} />
      </CardContent>

      <CardFooter className="justify-between gap-2">
        {selectionMode && publishable ? (
          <Field orientation="horizontal" className="w-auto">
            <Checkbox
              id={`select-${activity.garminActivityId}`}
              checked={selected}
              disabled={controlsDisabled}
              onCheckedChange={(checked) => onSelectedChange(checked === true)}
            />
            <FieldLabel htmlFor={`select-${activity.garminActivityId}`}>
              Select
            </FieldLabel>
          </Field>
        ) : publishable ? (
          <Button
            className="h-10"
            disabled={controlsDisabled || connectionsBroken}
            aria-busy={publishing}
            title={connectionsBroken ? "Reconnect Garmin and Strava to publish" : undefined}
            onClick={() =>
              runAction({
                type: "publish",
                activityId: activity.garminActivityId,
              })
            }
          >
            {publishing ? (
              <>
                <Spinner data-icon="inline-start" aria-hidden="true" />
                <span className="text-xs">Publishing…</span>
              </>
            ) : activity.publishStatus === "missing" ? (
              "Republish"
            ) : (
              "Publish"
            )}
          </Button>
        ) : (
          <span />
        )}

        <Dialog open={dialogOpen} onOpenChange={setOpen}>
          <DialogTrigger asChild>
            <Button variant="outline" className="h-10">
              Details
            </Button>
          </DialogTrigger>
          <DialogContent className="max-h-[calc(100dvh-2rem)] overflow-y-auto sm:max-w-2xl">
            <DialogHeader>
              <DialogTitle>{activity.title}</DialogTitle>
              <DialogDescription>
                {activity.startDateDisplay} at {activity.startClockDisplay} ·{" "}
                {formatActivityType(activity.activityType)}
              </DialogDescription>
            </DialogHeader>

            {editFeedback ? (
              <Alert variant={editFeedback.kind === "error" ? "destructive" : "default"}>
                {editFeedback.kind === "error" ? (
                  <TriangleAlertIcon />
                ) : (
                  <CheckCircle2Icon />
                )}
                <AlertTitle>
                  {editFeedback.kind === "error" ? "Couldn’t save" : "Changes saved"}
                </AlertTitle>
                <AlertDescription>{editFeedback.message}</AlertDescription>
              </Alert>
            ) : null}

            {editing ? (
              <form id={`edit-${activity.garminActivityId}`} onSubmit={saveActivity}>
                <FieldGroup>
                  <Field data-invalid={editFeedback?.kind === "error" && !title.trim()}>
                    <FieldLabel htmlFor={`title-${activity.garminActivityId}`}>
                      Title
                    </FieldLabel>
                    <Input
                      id={`title-${activity.garminActivityId}`}
                      className="h-11"
                      value={title}
                      disabled={saving}
                      aria-invalid={editFeedback?.kind === "error" && !title.trim()}
                      onChange={(event) => setTitle(event.target.value)}
                    />
                    {!title.trim() ? (
                      <FieldError>Activity title cannot be blank</FieldError>
                    ) : null}
                  </Field>
                  <Field>
                    <FieldLabel htmlFor={`description-${activity.garminActivityId}`}>
                      Description
                    </FieldLabel>
                    <Textarea
                      id={`description-${activity.garminActivityId}`}
                      rows={4}
                      value={description}
                      disabled={saving}
                      onChange={(event) => setDescription(event.target.value)}
                    />
                  </Field>
                </FieldGroup>
              </form>
            ) : (
              <div className="grid gap-5">
                {metrics.length > 0 ? <MetricGrid metrics={metrics} /> : null}
                <div className="grid gap-1">
                  <h4 className="font-medium">Description</h4>
                  <p className="leading-6 text-muted-foreground">
                    {activity.description || "No description yet."}
                  </p>
                </div>
                <ActivityLinks activity={activity} />
              </div>
            )}

            <DialogFooter className="flex-col sm:flex-row">
              {editing ? (
                <>
                  <Button
                    type="button"
                    variant="outline"
                    disabled={saving}
                    onClick={() => {
                      setEditing(false);
                      setTitle(activity.title);
                      setDescription(activity.description);
                      setEditFeedback(null);
                    }}
                  >
                    Cancel
                  </Button>
                  <Button
                    type="submit"
                    form={`edit-${activity.garminActivityId}`}
                    disabled={saving}
                    aria-busy={saving}
                  >
                    {saving ? (
                      <>
                        <Spinner data-icon="inline-start" aria-hidden="true" />
                        Saving…
                      </>
                    ) : (
                      "Save changes"
                    )}
                  </Button>
                </>
              ) : (
                <>
                  <Button
                    type="button"
                    variant="outline"
                    disabled={controlsDisabled}
                    onClick={() => setEditing(true)}
                  >
                    Edit
                  </Button>
                  {activity.publishStatus === "excluded" ? (
                    <Button
                      type="button"
                      variant="outline"
                      disabled={controlsDisabled}
                      aria-busy={restoring}
                      onClick={() =>
                        runAction({
                          type: "restore",
                          activityId: activity.garminActivityId,
                        })
                      }
                    >
                      {restoring ? (
                        <>
                          <Spinner data-icon="inline-start" aria-hidden="true" />
                          Restoring…
                        </>
                      ) : (
                        "Restore"
                      )}
                    </Button>
                  ) : publishable ? (
                    <Button
                      type="button"
                      variant="outline"
                      disabled={controlsDisabled}
                      aria-busy={excluding}
                      onClick={() =>
                        runAction({
                          type: "exclude",
                          activityId: activity.garminActivityId,
                        })
                      }
                    >
                      {excluding ? (
                        <>
                          <Spinner data-icon="inline-start" aria-hidden="true" />
                          Excluding…
                        </>
                      ) : (
                        "Exclude"
                      )}
                    </Button>
                  ) : null}
                  {publishable ? (
                    <Button
                      type="button"
                      disabled={controlsDisabled || connectionsBroken}
                      aria-busy={publishing}
                      onClick={() =>
                        runAction({
                          type: "publish",
                          activityId: activity.garminActivityId,
                        })
                      }
                    >
                      {publishing ? (
                        <>
                          <Spinner data-icon="inline-start" aria-hidden="true" />
                          <span className="text-xs">Publishing…</span>
                        </>
                      ) : activity.publishStatus === "missing" ? (
                        "Republish"
                      ) : (
                        "Publish"
                      )}
                    </Button>
                  ) : null}
                </>
              )}
            </DialogFooter>
          </DialogContent>
        </Dialog>
      </CardFooter>
    </Card>
  );
}

function MetricGrid({ metrics }: { metrics: Array<[string, string]> }) {
  return (
    <dl className="grid grid-cols-2 gap-x-4 gap-y-2 sm:grid-cols-4">
      {metrics.map(([label, value]) => (
        <div key={label} className="grid gap-0.5">
          <dt className="text-xs text-muted-foreground">{label}</dt>
          <dd className="font-medium tabular-nums">{value}</dd>
        </div>
      ))}
    </dl>
  );
}

function ActivityLinks({ activity }: { activity: Activity }) {
  return (
    <div className="flex flex-wrap gap-2" aria-label="Activity links">
      <Button asChild variant="ghost" size="sm" className="h-9 px-2">
        <a href={activity.garminUrl} target="_blank" rel="noreferrer">
          Garmin
          <ExternalLinkIcon data-icon="inline-end" />
        </a>
      </Button>
      {activity.stravaUrl ? (
        <Button asChild variant="ghost" size="sm" className="h-9 px-2">
          <a href={activity.stravaUrl} target="_blank" rel="noreferrer">
            Strava
            <ExternalLinkIcon data-icon="inline-end" />
          </a>
        </Button>
      ) : null}
    </div>
  );
}

function activityMetrics(activity: Activity): Array<[string, string]> {
  const detail = activity.detail;
  return [
    ["Distance", detail.distance],
    ["Duration", detail.duration],
    ["Moving time", detail.movingTime],
    ["Elapsed time", detail.elapsedTime],
    ["Pace", detail.pace],
    ["Speed", detail.speed],
    ["Elevation gain", detail.elevGain],
    ["Elevation loss", detail.elevLoss],
    ["Calories", detail.calories],
    ["Average HR", detail.avgHr],
    ["Maximum HR", detail.maxHr],
    ["Average power", detail.avgPower],
    ["Maximum power", detail.maxPower],
    ["Normalized power", detail.normPower],
    ["Aerobic TE", detail.aerobicTe],
    ["Anaerobic TE", detail.anaerobicTe],
    ["Training load", detail.trainingLoad],
    ["Average cadence", detail.avgCadence],
    ["Maximum cadence", detail.maxCadence],
    ["Sets", detail.totalSets],
    ["Reps", detail.totalReps],
    ["Volume", detail.totalVolume],
  ].filter((metric): metric is [string, string] => Boolean(metric[1]));
}

function activitySummaryMetrics(activity: Activity): Array<[string, string]> {
  const detail = activity.detail;
  return [
    ["Distance", detail.distance],
    ["Duration", detail.duration],
    ["Average HR", detail.avgHr],
    ["Sets", detail.totalSets],
    ["Reps", detail.totalReps],
  ].filter((metric): metric is [string, string] => Boolean(metric[1]));
}

function formatActivityType(activityType: string) {
  return activityType.replaceAll("_", " ");
}

function isPublishable(status: PublishStatus) {
  return status === "pending" || status === "held" || status === "missing";
}

function isPendingAction(
  state: ActivityActionState,
  type: "publish" | "exclude" | "restore" | "edit",
  activityId: number,
) {
  return (
    state.isPending &&
    state.action?.type === type &&
    state.action.activityId === activityId
  );
}
