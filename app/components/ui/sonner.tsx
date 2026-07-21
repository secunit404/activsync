"use client"

import { CheckIcon, Loader2Icon, TriangleAlertIcon, XIcon } from "lucide-react"
import { Toaster as Sonner, type ToasterProps } from "sonner"

/**
 * Toast restyle (design handoff frame `5b`). `sonner` (already a
 * dependency) supplies the queue, auto-dismiss timers, and manual close the
 * frame describes — this file only maps that onto the app's dark palette
 * and the mobile shell's fixed-bottom slot. Card colors/spacing live in
 * `app.css`, unlayered against sonner's own `[data-sonner-*]` attribute
 * hooks (see that file's comment for why `toastOptions.classNames` alone
 * can't win the cascade here). What stays here is what sonner's own
 * documented props already cover:
 *
 * - `offset`/`mobileOffset`: desktop's 24px bottom-right margin per the
 *   frame, and the mobile slot's 16px sides with a bottom clearance that
 *   tracks whichever of `AppTabBar`/`BulkActionBar` currently occupies the
 *   fixed-bottom slot (`--bottom-bar-height`, set in `app.css` and kept
 *   live by `BulkActionBar` — see its docstring). Sonner's own
 *   `@media (max-width:600px)` block already forces the single
 *   full-width/centered mobile layout the frame wants, regardless of
 *   `data-x-position` — that breakpoint doesn't line up with this app's
 *   `md:` (768px) shell breakpoint, but the 600–767px gap doesn't
 *   correspond to any of this app's three real layouts, so it's left as
 *   sonner's default rather than re-declaring its internal mobile ruleset
 *   a second time at 768px.
 * - `--width`: 340px, via the `style` prop sonner reads for its own
 *   `var(--width)` rule (only applies above 600px — sonner's mobile block
 *   forces `width: 100%` unconditionally).
 * - `icons`: the three variants the frame shows (success check, info/
 *   syncing spinner, error `!`), plus `warning` (used by
 *   `useActivityActions` for partial bulk-publish/exclude failures — not in
 *   the frame, but this app already fires it) and `loading`, each in its
 *   own tinted 22px circle per the frame. `close` swaps sonner's built-in
 *   glyph for the app's lucide `X` so it inherits `currentColor` from the
 *   `[data-close-button]` color set in `app.css`.
 *
 * The `ref` callback adds `role="status"` to the toaster's root `<section>`
 * (which sonner forwards its ref straight onto). Sonner's own DOM never
 * carries an explicit ARIA role — only `aria-live="polite"` on that
 * section — so without this, nothing here is announced as a live region by
 * role (`aria-live` alone doesn't imply `role="status"`), and
 * `getByRole("status")` (screen readers, and `e2e/toast.spec.ts`) would
 * find nothing. `role="status"` also implies `aria-live="polite"`, so this
 * doesn't conflict with what sonner already sets.
 */
const Toaster = ({ theme = "dark", ...props }: ToasterProps) => {
  return (
    <Sonner
      theme={theme}
      className="toaster group"
      duration={5000}
      offset={{ bottom: 24, right: 24 }}
      mobileOffset={{
        bottom: "calc(var(--bottom-bar-height, 74px) + 16px)",
        left: 16,
        right: 16,
      }}
      icons={{
        success: (
          <span className="grid size-full place-items-center rounded-full bg-success/15 text-success">
            <CheckIcon className="size-3" strokeWidth={3} />
          </span>
        ),
        info: (
          <span className="grid size-full place-items-center rounded-full bg-info/15 text-info">
            <Loader2Icon className="size-3 animate-spin" />
          </span>
        ),
        warning: (
          <span className="grid size-full place-items-center rounded-full bg-warning/15 text-warning">
            <TriangleAlertIcon className="size-3" />
          </span>
        ),
        error: (
          <span className="grid size-full place-items-center rounded-full bg-destructive/15 text-destructive">
            <span aria-hidden="true" className="text-[11px] leading-none font-extrabold">
              !
            </span>
          </span>
        ),
        loading: (
          <span className="grid size-full place-items-center rounded-full bg-info/15 text-info">
            <Loader2Icon className="size-3 animate-spin" />
          </span>
        ),
        close: <XIcon className="size-3.5" />,
      }}
      style={
        {
          "--normal-bg": "var(--popover)",
          "--normal-text": "var(--popover-foreground)",
          "--normal-border": "var(--border)",
          "--border-radius": "var(--radius)",
          "--width": "340px",
        } as React.CSSProperties
      }
      toastOptions={{
        classNames: {
          toast: "cn-toast",
        },
      }}
      ref={(node) => {
        node?.setAttribute("role", "status")
      }}
      {...props}
    />
  )
}

export { Toaster }
