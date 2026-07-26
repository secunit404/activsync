"""Safe, user-configurable Hevy activity-description rendering."""

from __future__ import annotations

import re
from datetime import datetime, timezone
from string import Formatter

DEFAULT_TITLE_TEMPLATE = "{clean_title}"

LEGACY_DEFAULT_TEMPLATE = (
    "🏋️ {title}\n"
    "{duration}\n"
    "{calories}\n"
    "{avg_hr}\n\n"
    "{exercises}\n\n"
    "{marker}"
)

DEFAULT_TEMPLATE = (
    "{duration}\n"
    "{calories}\n"
    "{avg_hr}\n\n"
    "{exercises}\n\n"
    "{marker}"
)

TITLE_FIELDS = frozenset({"title", "clean_title"})
DESCRIPTION_FIELDS = frozenset(
    {
        "duration",
        "calories",
        "avg_hr",
        "exercises",
        "marker",
    }
)
_EMOJI_CODEPOINTS = re.compile(
    "["
    "\U0001F1E6-\U0001FAFF"  # flags, pictographs, faces, symbols
    "\u2300-\u23FF"  # technical emoji such as hourglass/watch
    "\u2600-\u27BF"  # miscellaneous symbols and dingbats
    "]"
)


def clean_title(value: object) -> str:
    """Remove emoji presentation characters while preserving ordinary text."""
    title = str(value or "")
    title = _EMOJI_CODEPOINTS.sub("", title)
    title = title.replace("\ufe0e", "").replace("\ufe0f", "")
    title = title.replace("\u200d", "").replace("\u20e3", "")
    return " ".join(title.split()).strip() or "Workout"


def _validate_template(template: str, allowed_fields: frozenset[str], label: str) -> None:
    """Reject unknown or executable-looking formatting expressions."""
    if not template.strip():
        raise ValueError(f"{label} template cannot be empty.")
    try:
        parts = Formatter().parse(template)
        for _literal, field_name, format_spec, conversion in parts:
            if field_name is None:
                continue
            if field_name not in allowed_fields:
                raise ValueError(
                    f"Unknown {label.lower()} placeholder: {{{field_name}}}."
                )
            if format_spec or conversion:
                raise ValueError(f"{label} placeholders cannot use formatting options.")
    except ValueError as exc:
        if label.lower() in str(exc).lower():
            raise
        raise ValueError(f"Invalid {label.lower()} template: {exc}") from exc


def validate_title_template(template: str) -> None:
    _validate_template(template, TITLE_FIELDS, "Title")


def validate_template(template: str) -> None:
    _validate_template(template, DESCRIPTION_FIELDS, "Description")


def generate_title(workout: dict, *, template: str | None = None) -> str:
    chosen = template or DEFAULT_TITLE_TEMPLATE
    validate_title_template(chosen)
    title = workout.get("title", "Workout")
    return chosen.format_map(
        {"title": title, "clean_title": clean_title(title)}
    ).strip() or "Workout"


def _duration_minutes(workout: dict) -> int:
    def parse(value) -> datetime | None:
        if not isinstance(value, str) or not value:
            return None
        try:
            parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
        except ValueError:
            return None
        if parsed.tzinfo is None:
            parsed = parsed.replace(tzinfo=timezone.utc)
        return parsed.astimezone(timezone.utc)

    start = parse(workout.get("start_time"))
    end = parse(workout.get("end_time"))
    if start is None or end is None:
        return 0
    return max(0, int((end - start).total_seconds()) // 60)


def _exercise_summary(workout: dict) -> str:
    lines: list[str] = []
    for exercise in workout.get("exercises", []) or []:
        name = exercise.get("title") or exercise.get("name", "Unknown")
        all_sets = exercise.get("sets", []) or []
        warmup = [item for item in all_sets if item.get("type") == "warmup"]
        working = [item for item in all_sets if item.get("type") != "warmup"]
        if working:
            n_label = "set" if len(working) == 1 else "sets"
            has_distance = any(item.get("distance_meters") for item in working)
            has_duration = any(item.get("duration_seconds") for item in working)
            has_weight = any(
                item.get("weight_kg") or item.get("weight") for item in working
            )
            if has_distance or (has_duration and not has_weight):
                total_distance = sum(
                    item.get("distance_meters", 0) or 0 for item in working
                )
                total_duration = sum(
                    item.get("duration_seconds", 0) or 0 for item in working
                )
                details = [f"{len(working)} {n_label}"]
                if total_distance > 0:
                    details.append(f"{total_distance / 1000:.1f}km")
                if total_duration > 0:
                    details.append(f"{int(total_duration // 60)}min")
            else:
                weights = [
                    item.get("weight_kg") or item.get("weight", 0)
                    for item in working
                ]
                reps = [item.get("reps", 0) or 0 for item in working]
                details = [
                    f"{len(working)} {n_label}",
                    f"{max(weights) if weights else 0:.1f}kg × "
                    f"{max(reps) if reps else 0}",
                ]
            if warmup:
                details.append(f"{len(warmup)} warmup")
            for set_type in ("dropset", "failure"):
                count = sum(item.get("type") == set_type for item in working)
                if count:
                    details.append(f"{count} {set_type}")
            for key, label in (("rpe", "RPE"), ("custom_metric", "metric")):
                values = []
                for set_data in all_sets:
                    value = set_data.get(key)
                    if value is not None and value not in values:
                        values.append(value)
                if values:
                    details.append(f"{label} {', '.join(str(value) for value in values)}")
            lines.append(f"• {name}: {' · '.join(details)}")
        elif warmup:
            label = "set" if len(warmup) == 1 else "sets"
            lines.append(f"• {name}: {len(warmup)} warmup {label}")
    return "\n".join(lines)


def generate_description(
    workout: dict,
    calories: int | None = None,
    avg_hr: int | None = None,
    *,
    template: str | None = None,
) -> str:
    chosen = template or DEFAULT_TEMPLATE
    validate_template(chosen)
    minutes = _duration_minutes(workout)
    values = {
        "duration": f"⏱️ {minutes} min" if minutes > 0 else "",
        "calories": f"🔥 {calories} kcal" if calories else "",
        "avg_hr": f"❤️ avg {avg_hr} bpm" if avg_hr else "",
        "exercises": _exercise_summary(workout),
        "marker": "— synced by activsync",
    }
    rendered = chosen.format_map(values)
    # Optional metric placeholders become empty lines. Collapse those without
    # otherwise rewriting the user's wording or intentional single spacing.
    cleaned: list[str] = []
    for raw_line in rendered.splitlines():
        line = raw_line.rstrip()
        if line or (cleaned and cleaned[-1] != ""):
            cleaned.append(line)
    while cleaned and cleaned[-1] == "":
        cleaned.pop()
    return "\n".join(cleaned)
