import * as React from "react"

import { cn } from "@/lib/utils"

// Diverges from stock shadcn in one place: the fill is `background`, not
// `input/30`. A multi-line editor is a well, and stock's translucent control
// fill made it the brightest thing on screen whenever it sat on a `muted`
// panel. The `disabled:bg-input/50` and `dark:disabled:bg-input/80` overrides
// went with it — both were lighter still, so a disabled field outshone an
// active one. `disabled:opacity-50` already carries the disabled state.
function Textarea({ className, ...props }: React.ComponentProps<"textarea">) {
  return (
    <textarea
      data-slot="textarea"
      className={cn(
        "flex field-sizing-content min-h-16 w-full rounded-lg border border-input bg-transparent px-2.5 py-2 text-base transition-colors outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-50 aria-invalid:border-destructive aria-invalid:ring-3 aria-invalid:ring-destructive/20 md:text-sm dark:bg-background dark:aria-invalid:border-destructive/50 dark:aria-invalid:ring-destructive/40",
        className
      )}
      {...props}
    />
  )
}

export { Textarea }
