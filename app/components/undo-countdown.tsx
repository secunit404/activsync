import { useEffect, useState } from "react";

export function UndoCountdown({
  deadline,
  windowMs,
  verb = "Sends",
}: {
  deadline: number;
  windowMs: number;
  verb?: string;
}) {
  const [now, setNow] = useState(() => Date.now());
  const [drainMs] = useState(() => Math.max(0, deadline - Date.now()));
  useEffect(() => {
    const interval = window.setInterval(() => setNow(Date.now()), 250);
    return () => window.clearInterval(interval);
  }, []);
  const remainingMs = Math.max(0, deadline - now);

  return (
    <span className="grid w-full gap-2">
      {/* The ticking number stays out of the live region, or every second
          would be re-announced; the static sentence below is read once. */}
      <span aria-hidden="true" className="font-mono tabular-nums">
        {verb} in {Math.ceil(remainingMs / 1000)}s
      </span>
      <span className="sr-only">
        {verb} in {windowMs / 1000} seconds unless you undo.
      </span>
      <span
        aria-hidden="true"
        className="block h-0.5 overflow-hidden rounded-full bg-border motion-reduce:hidden"
      >
        <span
          className="block h-full origin-left animate-[publish-undo-drain_linear_forwards] bg-primary"
          style={{ animationDuration: `${drainMs}ms` }}
        />
      </span>
    </span>
  );
}
