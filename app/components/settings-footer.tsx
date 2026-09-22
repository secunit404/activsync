import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";

/**
 * The single Discard/Save pair shared by every dirty-trackable section
 * (Preferences, Auto-sync by type, Hevy integration — Connections acts
 * immediately via its own dialogs, so it isn't part of this). Both buttons
 * disable together when nothing is pending, so there's nothing to discard
 * either.
 */
export function SettingsFooter({
  dirty,
  saving,
  onDiscard,
  onSave,
}: {
  dirty: boolean;
  saving: boolean;
  onDiscard: () => void;
  onSave: () => void;
}) {
  return (
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
  );
}
