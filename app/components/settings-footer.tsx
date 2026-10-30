import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { cn } from "@/lib/utils";

/**
 * The single Discard/Save pair shared by every dirty-trackable section
 * (Preferences, Auto-sync by type, Hevy integration — Connections acts
 * immediately via its own dialogs, so it isn't part of this). Both buttons
 * disable together when nothing is pending, so there's nothing to discard
 * either.
 *
 * While changes are pending the footer sticks to the bottom of the viewport
 * (above the phone tab bar, which floats 12px up and is 58px tall) so the
 * unsaved state is visible without scrolling. `serverChangedUnderDraft` adds
 * a quiet notice when a refetch moved a field the owner is also editing.
 */
export function SettingsFooter({
  dirty,
  saving,
  serverChangedUnderDraft = false,
  onDiscard,
  onSave,
}: {
  dirty: boolean;
  saving: boolean;
  serverChangedUnderDraft?: boolean;
  onDiscard: () => void;
  onSave: () => void;
}) {
  return (
    <div
      data-testid="settings-footer"
      data-dirty={dirty}
      className={cn(
        "grid gap-2.5",
        dirty &&
          "sticky bottom-[calc(5.25rem+env(safe-area-inset-bottom))] z-10 rounded-[22px] border border-border/80 bg-rail p-3 shadow-[0_10px_30px_rgb(0_0_0/45%)] md:bottom-6 md:ml-auto md:w-fit md:rounded-xl",
      )}
    >
      {dirty && serverChangedUnderDraft ? (
        <p role="status" className="text-sm text-muted-foreground">
          Some of these settings changed on the server while you were editing.
          Saving keeps your version.
        </p>
      ) : null}
      <div className="flex gap-2.5 sm:justify-end">
        <Button
          type="button"
          variant="outline"
          className="h-11 flex-1 sm:flex-none"
          disabled={!dirty || saving}
          onClick={onDiscard}
        >
          Discard
        </Button>
        <Button
          type="button"
          className="h-11 flex-1 sm:flex-none"
          disabled={!dirty || saving}
          onClick={onSave}
        >
          {saving ? <Spinner /> : null}
          Save
        </Button>
      </div>
    </div>
  );
}
