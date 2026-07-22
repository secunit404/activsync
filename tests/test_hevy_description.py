import pytest

from activsync.hevy_description import (
    clean_title,
    generate_description,
    validate_template,
)


WORKOUT = {
    "title": "Push day",
    "start_time": "2026-07-22T10:00:00Z",
    "end_time": "2026-07-22T11:05:00Z",
    "exercises": [
        {
            "title": "Bench Press (Barbell)",
            "sets": [{"type": "normal", "reps": 8, "weight_kg": 80}],
        }
    ],
}


def test_custom_template_controls_order_and_wording():
    description = generate_description(
        WORKOUT,
        template="{title}\n{exercises}\nDuration: {duration}\nMade with ActivSync",
    )

    assert description == (
        "Push day\n"
        "• Bench Press (Barbell): 1 set · 80.0kg × 8\n"
        "Duration: ⏱️ 65 min\n"
        "Made with ActivSync"
    )


def test_empty_optional_metrics_do_not_leave_large_blank_gaps():
    description = generate_description(
        WORKOUT,
        template="{title}\n{calories}\n{avg_hr}\n\n{exercises}",
    )

    assert description == "Push day\n\n• Bench Press (Barbell): 1 set · 80.0kg × 8"


def test_template_rejects_unknown_fields_and_format_expressions():
    with pytest.raises(ValueError, match="Unknown description placeholder"):
        validate_template("{secret}")
    with pytest.raises(ValueError, match="formatting options"):
        validate_template("{title!r}")


def test_clean_title_placeholder_removes_emoji_but_keeps_hevy_text():
    workout = {**WORKOUT, "title": "🔥 Push day 💪🏽 — A/B"}

    assert clean_title(workout["title"]) == "Push day — A/B"
    assert generate_description(workout, template="{clean_title}") == "Push day — A/B"


def test_clean_title_falls_back_when_title_contains_only_emoji():
    assert clean_title("🔥💪") == "Workout"
