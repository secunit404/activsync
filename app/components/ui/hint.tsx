import { InfoIcon } from "lucide-react";
import { Popover as PopoverPrimitive } from "radix-ui";
import { useState, type ReactNode } from "react";

import { cn } from "@/lib/utils";

/**
 * The explanatory note for a setting, folded into an icon beside its label so
 * the form reads as a list of controls rather than a wall of prose.
 *
 * Built on Popover, not Tooltip. A tooltip is the usual answer for this and is
 * the wrong one here: Radix tooltips open on hover and focus only, and a touch
 * device has neither — every description would simply be unreachable on the
 * phone, which is where this app mostly gets used. A popover opens on tap,
 * click and keyboard everywhere, and hover is added back on top for pointers
 * that actually have it, so a desktop user still gets the tooltip behaviour.
 *
 * The trigger is a real button carrying the setting's name, so a screen reader
 * reads "About Garmin poll interval, button" rather than an unlabelled icon,
 * and the note is announced when it opens.
 */
export function Hint({
  label,
  children,
  className,
}: {
  /** The setting this explains — becomes part of the trigger's name. */
  label: string;
  children: ReactNode;
  className?: string;
}) {
  const [open, setOpen] = useState(false);

  // `pointerType` is the reliable signal: a coarse pointer still fires
  // `pointerenter` on tap, which would race the click into a instant
  // open-then-close.
  const hoverOnly = (event: { pointerType: string }, next: boolean) => {
    if (event.pointerType !== "touch") {
      setOpen(next);
    }
  };

  return (
    <PopoverPrimitive.Root open={open} onOpenChange={setOpen}>
      <PopoverPrimitive.Trigger
        type="button"
        aria-label={`About ${label}`}
        onPointerEnter={(event) => hoverOnly(event, true)}
        onPointerLeave={(event) => hoverOnly(event, false)}
        className={cn(
          // 24px box around an 14px glyph: the icon stays visually secondary
          // while the tap target does not.
          "grid size-6 shrink-0 place-items-center rounded-full text-muted-foreground transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none data-[state=open]:text-primary",
          className,
        )}
      >
        <InfoIcon className="size-[14px]" aria-hidden="true" />
      </PopoverPrimitive.Trigger>
      <PopoverPrimitive.Portal>
        <PopoverPrimitive.Content
          data-testid="hint-content"
          side="top"
          align="start"
          sideOffset={6}
          collisionPadding={16}
          // A hover-opened popover must not steal focus, or the page jumps
          // under the cursor. Tap and keyboard still move focus normally.
          onOpenAutoFocus={(event) => event.preventDefault()}
          className="z-50 max-w-[min(20rem,calc(100vw-2rem))] rounded-lg border border-border bg-popover px-3 py-2 text-sm leading-normal text-muted-foreground shadow-[0_18px_40px_rgb(0_0_0/45%)]"
        >
          {children}
          <PopoverPrimitive.Arrow className="fill-popover" />
        </PopoverPrimitive.Content>
      </PopoverPrimitive.Portal>
    </PopoverPrimitive.Root>
  );
}
