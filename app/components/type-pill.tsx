/**
 * Table (Task 8) TYPE column label. Garmin activity type keys can be long
 * (`stand_up_paddleboarding_v2` is the stress case named in the design
 * handoff), so the visible label truncates with an ellipsis while the raw
 * value stays discoverable via `title`.
 *
 * No chip fill: this sits in its own column under a TYPE header, so the
 * column already says what the value is. A grey chip on a grey row only
 * added a second box for the eye to parse.
 */
export function TypePill({ type }: { type: string }) {
  return (
    <span
      className="inline-block max-w-[10rem] truncate font-mono text-[11px] text-muted-foreground"
      title={type}
    >
      {formatTypeLabel(type)}
    </span>
  );
}

export function formatTypeLabel(type: string) {
  return type.replaceAll("_", " ").toUpperCase();
}
