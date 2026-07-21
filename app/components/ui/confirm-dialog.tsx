import { useRef } from "react";

import { Button } from "@/components/ui/button";
import { ResponsiveOverlay } from "@/components/ui/responsive-overlay";
import { Spinner } from "@/components/ui/spinner";

export type ConfirmDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** The accessible name — keep it short, the header truncates at one line. */
  title: string;
  /**
   * The accessible description — this is the actual confirmation message.
   * Unlike the title, it wraps rather than truncating, so put any
   * variable-length content (an item's name, etc.) here.
   */
  description: string;
  confirmLabel: string;
  /** Shown on the confirm button, in place of `confirmLabel`, while `pending`. */
  pendingLabel: string;
  cancelLabel?: string;
  pending: boolean;
  onConfirm: () => void;
};

/**
 * A destructive-action confirmation, built on `ResponsiveOverlay` so it
 * inherits the same focus trap, Escape handling, and desktop-modal /
 * mobile-sheet presentation as every other overlay in the app — replacing
 * `window.confirm()`, which is unstyled and hangs Playwright's CDP browser
 * (so the native dialog could never be exercised end to end).
 *
 * Escape, backdrop click, and the header close button all route through
 * `onOpenChange(false)` exactly like every other `ResponsiveOverlay`
 * consumer — none of those paths can reach `onConfirm`. Only the destructive
 * button in the footer calls it, and it is disabled while `pending` so a
 * slow disconnect/resync can't be double-fired by a second click.
 */
export function ConfirmDialog({
  open,
  onOpenChange,
  title,
  description,
  confirmLabel,
  pendingLabel,
  cancelLabel = "Cancel",
  pending,
  onConfirm,
}: ConfirmDialogProps) {
  const footerRef = useRef<HTMLDivElement>(null);

  return (
    <ResponsiveOverlay
      open={open}
      onOpenChange={onOpenChange}
      title={title}
      description={description}
      mobile="sheet"
      role="alertdialog"
      onOpenAutoFocus={(event) => {
        // Radix's default is to focus the first focusable element — here
        // that would be the destructive button. Focus the safe action
        // instead: querying by testid (rather than adding a ref prop to
        // `Button`'s public API) matches the pattern the MFA sheet already
        // established for this same "override Radix's default focus" case.
        event.preventDefault();
        footerRef.current
          ?.querySelector<HTMLButtonElement>('[data-testid="confirm-dialog-cancel"]')
          ?.focus();
      }}
      footer={
        <div
          ref={footerRef}
          className="flex flex-col-reverse gap-2.5 md:flex-row md:justify-end"
        >
          <Button
            type="button"
            variant="outline"
            className="h-11"
            data-testid="confirm-dialog-cancel"
            onClick={() => onOpenChange(false)}
          >
            {cancelLabel}
          </Button>
          <Button
            type="button"
            variant="destructive"
            className="h-11"
            disabled={pending}
            onClick={onConfirm}
          >
            {pending ? <Spinner data-icon="inline-start" aria-hidden="true" /> : null}
            {pending ? pendingLabel : confirmLabel}
          </Button>
        </div>
      }
    />
  );
}
