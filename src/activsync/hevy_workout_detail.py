"""Shared API presentation model for a Hevy workout payload."""

from __future__ import annotations

from activsync.api_routes import ApiModel
from activsync.hevy_apply import generate_description


class HevyWorkoutSetDetail(ApiModel):
    number: int
    set_type: str
    reps: float | None
    weight_kg: float | None
    distance_meters: float | None
    duration_seconds: float | None
    rpe: float | None
    custom_metric: str | float | None


class HevyWorkoutExerciseDetail(ApiModel):
    title: str
    notes: str | None
    template_id: str | None
    sets: list[HevyWorkoutSetDetail]


class HevyWorkoutDetail(ApiModel):
    hevy_id: str
    title: str
    start_time: str
    end_time: str
    notes: str | None
    exercises: list[HevyWorkoutExerciseDetail]
    description_preview: str


def _number(value) -> float | None:
    if value is None or value == "":
        return None
    try:
        return float(value)
    except (TypeError, ValueError):
        return None


def workout_detail(
    payload: dict,
    *,
    hevy_id: str,
    title: str,
    start_time: str,
    end_time: str,
    description_template: str | None,
) -> dict:
    """Return the same typed workout preview for queue and backfill rows."""
    exercises = []
    for exercise in payload.get("exercises", []) or []:
        sets = []
        for index, set_data in enumerate(exercise.get("sets", []) or [], start=1):
            custom_metric = set_data.get("custom_metric")
            if custom_metric is not None and not isinstance(
                custom_metric, (str, int, float)
            ):
                custom_metric = str(custom_metric)
            sets.append(
                {
                    "number": index,
                    "set_type": set_data.get("type") or "normal",
                    "reps": _number(set_data.get("reps")),
                    "weight_kg": _number(
                        set_data.get("weight_kg", set_data.get("weight"))
                    ),
                    "distance_meters": _number(set_data.get("distance_meters")),
                    "duration_seconds": _number(set_data.get("duration_seconds")),
                    "rpe": _number(set_data.get("rpe")),
                    "custom_metric": custom_metric,
                }
            )
        exercises.append(
            {
                "title": exercise.get("title")
                or exercise.get("name")
                or "Unknown exercise",
                "notes": exercise.get("notes") or None,
                "template_id": exercise.get("exercise_template_id") or None,
                "sets": sets,
            }
        )
    return {
        "hevy_id": hevy_id,
        "title": title or payload.get("title") or hevy_id,
        "start_time": payload.get("start_time") or start_time,
        "end_time": payload.get("end_time") or end_time,
        "notes": payload.get("description") or payload.get("notes") or None,
        "exercises": exercises,
        "description_preview": generate_description(
            payload,
            template=description_template,
        ),
    }
