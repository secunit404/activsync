/**
 * Shared toast timing (design handoff frame `5b`). A failure needs to be
 * read and, usually, acted on (retry, reconnect, check what changed) —
 * gone-in-5-seconds like a success confirmation risks the user never
 * catching what happened. `sonner.tsx` sets the app-wide default duration
 * (5000ms, for success/warning/info/loading) on the `<Toaster>` itself;
 * this is the one deliberate exception, passed explicitly at every
 * `toast.error(...)` call site instead of a second global default, since
 * sonner has no per-type duration on the `Toaster` — only a single global
 * default plus a per-call override.
 */
export const ERROR_TOAST_DURATION_MS = 8000;
