import { useRef, useState, type ReactNode } from "react";
import { Dialog as DialogPrimitive } from "radix-ui";
import { XIcon } from "lucide-react";

import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogClose,
  DialogDescription,
  DialogOverlay,
  DialogPortal,
  DialogTitle,
} from "@/components/ui/dialog";

export type ResponsiveOverlayProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Required: this is the dialog's accessible name as well as its visible heading. */
  title: string;
  description?: ReactNode;
  /** How the overlay presents below `md` (768px). Desktop is always a centered modal. */
  mobile?: "cover" | "sheet";
  /**
   * Desktop width. `wide` suits list-shaped bodies where the default 560px
   * makes every row truncate. No effect on mobile, which is full-width in
   * both presentations.
   */
  size?: keyof typeof DESKTOP_WIDTH_CLASS;
  /** Pinned to the bottom of the panel, above the scrolling body. */
  footer?: ReactNode;
  /**
   * By default this component focuses the panel itself on open, rather than
   * letting Radix focus the first focusable child (the close button, or a
   * form's first input — both of which flash a focus ring the user never
   * asked for). Pass this to focus something specific instead; it replaces
   * the default entirely, so call `event.preventDefault()` yourself.
   */
  onOpenAutoFocus?: (event: Event) => void;
  /**
   * `"alertdialog"` for a destructive confirmation — it interrupts to ask
   * for a decision, rather than presenting a plain content surface. Plain
   * `"dialog"` (the default) covers every other consumer.
   */
  role?: "dialog" | "alertdialog";
  /**
   * Optional: a confirmation dialog has nothing to put here — the question
   * lives in `title`/`description`. The scrolling body region (and its
   * padding) is only rendered when this is given.
   */
  children?: ReactNode;
};

/**
 * The desktop treatment is identical for both mobile variants: a centered modal
 * that matches where the app rail appears (`md:`, 768px — see Task 5), so the
 * two shells never disagree about what "desktop" means.
 */
const DESKTOP_CONTENT_CLASS =
  "md:inset-auto md:top-1/2 md:bottom-auto md:left-1/2 md:h-auto md:max-h-[85vh] md:w-[calc(100%-2rem)] md:-translate-x-1/2 md:-translate-y-1/2 md:rounded-xl md:border md:shadow-[var(--shadow-frame)] md:data-open:zoom-in-95 md:data-closed:zoom-out-95 md:data-open:slide-in-from-right-0 md:data-open:slide-in-from-bottom-0 md:data-closed:slide-out-to-right-0 md:data-closed:slide-out-to-bottom-0";

/**
 * Desktop max width. Mobile is unaffected — both mobile presentations are
 * full-width. `wide` exists for list-shaped bodies (the Hevy backfill
 * preview), where 560px forced every row to truncate.
 */
const DESKTOP_WIDTH_CLASS = {
  default: "md:max-w-[560px]",
  wide: "md:max-w-[760px]",
} as const;

/**
 * Mobile presentation. `cover` is a full-screen push in from the right (frames
 * 2a/2b); `sheet` is a bottom-anchored panel that slides up (frames 3c/4a).
 * Both are pure CSS — no `window.innerWidth` branch — so a resize is free and
 * the semantics stay testable in jsdom.
 */
const MOBILE_CONTENT_CLASS = {
  cover:
    "inset-0 h-full rounded-none data-open:slide-in-from-right data-closed:slide-out-to-right",
  // `translate-y-(--sheet-drag)` is what the drag gesture below moves. It is
  // 0px at rest, and `md:-translate-y-1/2` (from DESKTOP_CONTENT_CLASS) wins
  // above the breakpoint, so the desktop centering is never affected.
  sheet:
    "inset-x-0 bottom-0 max-h-[85vh] translate-y-(--sheet-drag) rounded-t-[20px] border-t data-open:slide-in-from-bottom data-closed:slide-out-to-bottom",
} as const;

/**
 * How far the sheet must be dragged down before release dismisses it rather
 * than springing back. Roughly a thumb's travel — short enough to feel light,
 * long enough that a stray swipe on the handle doesn't close the sheet.
 */
const SHEET_DISMISS_PX = 96;

export function ResponsiveOverlay({
  open,
  onOpenChange,
  title,
  description,
  mobile = "cover",
  size = "default",
  footer,
  onOpenAutoFocus,
  role = "dialog",
  children,
}: ResponsiveOverlayProps) {
  // Drag-to-dismiss lives on the grab handle only, which is `md:hidden` — so
  // desktop is excluded by CSS rather than by a `window.innerWidth` branch,
  // keeping this component's "no JS breakpoint" property intact. Handle-only
  // also means the gesture can never fight the body's own scrolling, which is
  // the failure mode that makes drag-to-dismiss hard to hand-roll.
  //
  // ponytail: no snap points and no velocity fling — a slow 96px drag
  // dismisses, a fast 40px flick does not. Swap in `vaul` if either matters.
  // The refs, not the state, are what the handlers read. `pointermove` is a
  // continuous-priority event in React, so a fast drag can batch every move
  // update without an intervening render — leaving the `pointerup` handler
  // looking at a stale `dragY` of 0 and silently refusing to dismiss. State
  // exists only to drive the render; the refs decide.
  const [dragY, setDragY] = useState(0);
  const [dragging, setDragging] = useState(false);
  const dragStartY = useRef<number | null>(null);
  const dragDistance = useRef(0);
  const contentRef = useRef<HTMLDivElement>(null);

  // A dismissing drag leaves `dragY` where the finger let go (see endDrag), so
  // the exit animation continues from there instead of snapping back first.
  // That offset has to be cleared before the next open, or the sheet slides in
  // already pushed down. Consumers that unmount on close never reach this;
  // ConfirmDialog, which stays mounted with `open={false}`, does.
  const [wasOpen, setWasOpen] = useState(open);
  if (open !== wasOpen) {
    setWasOpen(open);
    if (open && dragY !== 0) {
      setDragY(0);
    }
  }

  const startDrag = (event: React.PointerEvent<HTMLDivElement>) => {
    dragStartY.current = event.clientY;
    dragDistance.current = 0;
    setDragging(true);
    // Without capture the pointer leaves the handle on the first few pixels
    // and the rest of the gesture goes to whatever is underneath.
    event.currentTarget.setPointerCapture?.(event.pointerId);
  };

  const moveDrag = (event: React.PointerEvent<HTMLDivElement>) => {
    if (dragStartY.current === null) return;
    // Clamped at 0: the sheet is already at its maximum height, so dragging
    // up has nowhere to go and would just detach it from the bottom edge.
    dragDistance.current = Math.max(0, event.clientY - dragStartY.current);
    setDragY(dragDistance.current);
  };

  const endDrag = () => {
    if (dragStartY.current === null) return;
    dragStartY.current = null;
    const travelled = dragDistance.current;
    dragDistance.current = 0;
    setDragging(false);
    if (travelled > SHEET_DISMISS_PX) {
      // Deliberately NOT resetting `dragY` here: doing so snapped the sheet
      // back up to its resting position for one frame before the exit
      // animation started from there, which read as a flicker. Leaving the
      // translate in place lets the slide-out continue from where the finger
      // released. The open-transition reset above clears it afterwards.
      onOpenChange(false);
    } else {
      setDragY(0);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogPortal>
        <DialogOverlay
          data-slot="responsive-overlay-backdrop"
          className="bg-black/55 supports-backdrop-filter:backdrop-blur-[2px]"
        />
        <DialogPrimitive.Content
          data-slot="responsive-overlay"
          data-testid="responsive-overlay"
          data-mobile={mobile}
          // Radix always points aria-describedby at its generated description
          // id. With no description rendered that would dangle, so opt out by
          // passing the prop explicitly as undefined (it overrides the default
          // because our props spread last).
          {...(description ? {} : { "aria-describedby": undefined })}
          ref={contentRef}
          role={role}
          // Radix's default lands on the first focusable child — the close
          // button, or the first form field — so every overlay opened with a
          // visible focus ring on a control the user did not choose. Focus
          // the panel instead (Radix gives Content `tabindex="-1"`): the trap,
          // Escape, and Tab-into-the-dialog all still work, with no ring.
          onOpenAutoFocus={
            onOpenAutoFocus ??
            ((event) => {
              event.preventDefault();
              contentRef.current?.focus();
            })
          }
          style={{ "--sheet-drag": `${dragY}px` } as React.CSSProperties}
          className={cn(
            "fixed z-50 flex flex-col overflow-hidden border-border bg-card text-card-foreground outline-none",
            "duration-150 data-open:animate-in data-open:fade-in-0 data-closed:animate-out data-closed:fade-out-0",
            MOBILE_CONTENT_CLASS[mobile],
            DESKTOP_WIDTH_CLASS[size],
            DESKTOP_CONTENT_CLASS,
            // Off while the finger is down so the sheet tracks it exactly;
            // on after release so a short drag springs back instead of
            // snapping.
            //
            // `translate` specifically, never `transition-transform`: the
            // latter covers transform/translate/scale/rotate in Tailwind v4,
            // and `transform` is what the enter/exit keyframes animate — the
            // two fought, and the sheet opened stuck off-screen. The drag
            // uses the standalone `translate` property, so the two never
            // touch. No `duration-*` either; twMerge would collapse it into
            // the `duration-150` above and retime the animation.
            dragging
              ? "transition-none"
              : "transition-[translate] ease-out motion-reduce:transition-none",
          )}
        >
          {mobile === "sheet" && (
            // `touch-none` stops the browser claiming the vertical pan for a
            // scroll before the pointer handlers see it. The padding is the
            // touch target — the visible bar is only 4px tall.
            <div
              data-testid="responsive-overlay-handle"
              className="flex shrink-0 cursor-grab touch-none justify-center pt-3 pb-2.5 active:cursor-grabbing md:hidden"
              onPointerDown={startDrag}
              onPointerMove={moveDrag}
              onPointerUp={endDrag}
              onPointerCancel={endDrag}
            >
              <span
                aria-hidden="true"
                className="h-1 w-9 rounded-full bg-muted-foreground/40"
              />
            </div>
          )}
          <div className="flex shrink-0 items-start justify-between gap-4 border-b border-border px-5 py-4 md:px-7 md:py-6">
            <div className="flex min-w-0 flex-col gap-1.5">
              <DialogTitle className="truncate text-xl font-extrabold tracking-[-0.02em]">
                {title}
              </DialogTitle>
              {description && (
                <DialogDescription className="text-sm leading-relaxed">
                  {description}
                </DialogDescription>
              )}
            </div>
            <DialogClose asChild>
              <Button variant="outline" size="icon" className="shrink-0">
                <XIcon />
                <span className="sr-only">Close</span>
              </Button>
            </DialogClose>
          </div>
          {children != null && (
            <div
              data-testid="responsive-overlay-body"
              className="overlay-scroll min-h-0 flex-1 overflow-y-auto px-5 py-5 md:px-7 md:py-6"
            >
              {children}
            </div>
          )}
          {footer && (
            <div
              data-testid="responsive-overlay-footer"
              // The mobile bottom padding is deliberately larger than the top:
              // `env(safe-area-inset-bottom)` is 0 without `viewport-fit=cover`
              // (which this app does not set), so on a home-indicator phone the
              // buttons sat right on the gesture bar. 1.75rem is the clearance,
              // and the env() term still helps anywhere it does report a value.
              className="sticky bottom-0 z-10 shrink-0 border-t border-border bg-card px-5 pt-4 pb-[calc(1.75rem+env(safe-area-inset-bottom))] md:px-7 md:pt-[18px] md:pb-[18px]"
            >
              {footer}
            </div>
          )}
        </DialogPrimitive.Content>
      </DialogPortal>
    </Dialog>
  );
}
