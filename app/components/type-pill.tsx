/**
 * Table (Task 8) TYPE column pill. Garmin activity type keys can be long
 * (`stand_up_paddleboarding_v2` is the stress case named in the design
 * handoff), so the visible label truncates with an ellipsis while the raw
 * value stays discoverable via `title`.
 */
export function TypePill({ type }: { type: string }) {
  return (
    <span
      className="inline-block max-w-[10rem] truncate rounded-md bg-muted px-1.5 py-0.5 font-mono text-[11px] text-muted-foreground"
      title={type}
    >
      {formatTypeLabel(type)}
    </span>
  );
}

export function formatTypeLabel(type: string) {
  return type.replaceAll("_", " ").toUpperCase();
}
