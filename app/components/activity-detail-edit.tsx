import { Button } from "@/components/ui/button";
import { Field, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { Textarea } from "@/components/ui/textarea";
import type { Activity } from "@/lib/api";

const DESCRIPTION_MAX_LENGTH = 4000;

export type ActivityDetailEditProps = {
  activity: Activity;
  title: string;
  description: string;
  onTitleChange: (value: string) => void;
  onDescriptionChange: (value: string) => void;
  disabled: boolean;
};

/**
 * Body content for the activity detail overlay's edit mode (handoff frame
 * `2b`): the publish-aware info banner, then the title/description fields.
 *
 * The banner's wording is driven entirely by `activity.publishStatus` — the
 * business rule (spec §8) is that saving an already-published activity
 * writes to both Garmin and Strava, while anything not yet published writes
 * to Garmin only. `role="status"` (not `role="alert"`) because this is
 * informational, not an error — see the accessibility note in the brief.
 */
export function ActivityDetailEdit({
  activity,
  title,
  description,
  onTitleChange,
  onDescriptionChange,
  disabled,
}: ActivityDetailEditProps) {
  const published = activity.publishStatus === "published";
  const titleId = `activity-detail-title-${activity.garminActivityId}`;
  const descriptionId = `activity-detail-description-${activity.garminActivityId}`;

  return (
    <div className="grid gap-5 md:gap-[22px]">
      <div
        role="status"
        className="flex items-start gap-2.5 rounded-[10px] border border-info/25 bg-info/8 p-3 px-3.5"
      >
        <span aria-hidden="true" className="mt-[5px] size-[7px] shrink-0 rounded-full bg-info" />
        <span className="text-[13px] leading-relaxed text-info">
          {published ? (
            <>
              This activity is published to Strava — changes save to{" "}
              <b className="font-semibold text-foreground">both Garmin and Strava</b>.
            </>
          ) : (
            <>
              Not published to Strava yet — changes save to{" "}
              <b className="font-semibold text-foreground">Garmin only</b>. Once this activity is
              published, editing updates{" "}
              <b className="font-semibold text-foreground">both Garmin and Strava</b>.
            </>
          )}
        </span>
      </div>

      <Field>
        <FieldLabel
          htmlFor={titleId}
          className="font-mono text-xs tracking-[0.06em] text-muted-foreground uppercase"
        >
          Title
        </FieldLabel>
        <Input
          id={titleId}
          value={title}
          disabled={disabled}
          onChange={(event) => onTitleChange(event.target.value)}
        />
      </Field>

      <Field>
        <FieldLabel
          htmlFor={descriptionId}
          className="font-mono text-xs tracking-[0.06em] text-muted-foreground uppercase"
        >
          Description
        </FieldLabel>
        <Textarea
          id={descriptionId}
          rows={5}
          value={description}
          disabled={disabled}
          maxLength={DESCRIPTION_MAX_LENGTH}
          onChange={(event) => onDescriptionChange(event.target.value)}
        />
        <span className="self-end font-mono text-[11.5px] text-muted-foreground">
          {description.length} / {DESCRIPTION_MAX_LENGTH}
        </span>
      </Field>
    </div>
  );
}

export type ActivityDetailEditFooterProps = {
  dirty: boolean;
  saving: boolean;
  onCancel: () => void;
  onSave: () => void;
};

/**
 * Footer for edit mode: Cancel + Save, paired 50/50 on mobile and
 * right-aligned on desktop (frame 2b). Save stays disabled until `dirty` —
 * the route computes that by comparing the live field state against the
 * activity's saved title/description (see `activity-detail.tsx`).
 */
export function ActivityDetailEditFooter({
  dirty,
  saving,
  onCancel,
  onSave,
}: ActivityDetailEditFooterProps) {
  return (
    <div className="flex gap-2.5 *:flex-1 md:justify-end md:*:flex-none">
      <Button variant="outline" disabled={saving} onClick={onCancel}>
        Cancel
      </Button>
      <Button disabled={!dirty || saving} aria-busy={saving} onClick={onSave}>
        {saving ? (
          <>
            <Spinner data-icon="inline-start" aria-hidden="true" />
            Saving…
          </>
        ) : (
          "Save changes"
        )}
      </Button>
    </div>
  );
}
