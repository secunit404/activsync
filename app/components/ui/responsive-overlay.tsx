import type { ReactNode } from "react";
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
  description?: string;
  /** How the overlay presents below `md` (768px). Desktop is always a centered modal. */
  mobile?: "cover" | "sheet";
  /** Pinned to the bottom of the panel, above the scrolling body. */
  footer?: ReactNode;
  children: ReactNode;
};

/**
 * The desktop treatment is identical for both mobile variants: a centered modal
 * that matches where the app rail appears (`md:`, 768px — see Task 5), so the
 * two shells never disagree about what "desktop" means.
 */
const DESKTOP_CONTENT_CLASS =
  "md:inset-auto md:top-1/2 md:bottom-auto md:left-1/2 md:h-auto md:max-h-[85vh] md:w-[calc(100%-2rem)] md:max-w-[560px] md:-translate-x-1/2 md:-translate-y-1/2 md:rounded-xl md:border md:shadow-[var(--shadow-frame)] md:data-open:zoom-in-95 md:data-closed:zoom-out-95 md:data-open:slide-in-from-right-0 md:data-open:slide-in-from-bottom-0 md:data-closed:slide-out-to-right-0 md:data-closed:slide-out-to-bottom-0";

/**
 * Mobile presentation. `cover` is a full-screen push in from the right (frames
 * 2a/2b); `sheet` is a bottom-anchored panel that slides up (frames 3c/4a).
 * Both are pure CSS — no `window.innerWidth` branch — so a resize is free and
 * the semantics stay testable in jsdom.
 */
const MOBILE_CONTENT_CLASS = {
  cover:
    "inset-0 h-full rounded-none data-open:slide-in-from-right data-closed:slide-out-to-right",
  sheet:
    "inset-x-0 bottom-0 max-h-[85vh] rounded-t-[20px] border-t data-open:slide-in-from-bottom data-closed:slide-out-to-bottom",
} as const;

export function ResponsiveOverlay({
  open,
  onOpenChange,
  title,
  description,
  mobile = "cover",
  footer,
  children,
}: ResponsiveOverlayProps) {
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
          className={cn(
            "fixed z-50 flex flex-col overflow-hidden border-border bg-card text-card-foreground outline-none",
            "duration-150 data-open:animate-in data-open:fade-in-0 data-closed:animate-out data-closed:fade-out-0",
            MOBILE_CONTENT_CLASS[mobile],
            DESKTOP_CONTENT_CLASS,
          )}
        >
          {mobile === "sheet" && (
            <div className="flex shrink-0 justify-center pt-2.5 pb-1 md:hidden">
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
          <div
            data-testid="responsive-overlay-body"
            className="overlay-scroll min-h-0 flex-1 overflow-y-auto px-5 py-5 md:px-7 md:py-6"
          >
            {children}
          </div>
          {footer && (
            <div
              data-testid="responsive-overlay-footer"
              className="sticky bottom-0 z-10 shrink-0 border-t border-border bg-card px-5 pt-4 pb-[calc(1rem+env(safe-area-inset-bottom))] md:px-7 md:pt-[18px] md:pb-[18px]"
            >
              {footer}
            </div>
          )}
        </DialogPrimitive.Content>
      </DialogPortal>
    </Dialog>
  );
}
