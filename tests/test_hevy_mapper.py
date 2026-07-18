"""Tests for exercise mapping lookup and suggestions."""

import sqlite3

import pytest

from activsync import hevy_db, hevy_mapper
from activsync.hevy_mapper import (
    CATEGORY_NAMES,
    SUBCATEGORY_NAMES,
    UNKNOWN_CATEGORY,
    MappingMiss,
    lookup_exercise,
    suggest_mapping,
)


def make_conn():
    conn = sqlite3.connect(":memory:")
    conn.row_factory = sqlite3.Row
    hevy_db.init_schema(conn)
    return conn


# -- lookup_exercise --------------------------------------------------------

def test_template_id_hit():
    conn = make_conn()
    # "79D0BB3A" is Bench Press (Barbell) in the generated template map
    cat, sub, name = lookup_exercise(conn, "Bänkpress (Skivstång)", "79D0BB3A")
    assert (cat, sub) == (0, 1)
    assert name == "Bänkpress (Skivstång)"


def test_user_mapping_overrides_template_map():
    conn = make_conn()
    hevy_db.save_mapping(conn, "79D0BB3A", 24, 3)
    cat, sub, _ = lookup_exercise(conn, "Bench Press (Barbell)", "79D0BB3A")
    assert (cat, sub) == (24, 3)


def test_rejected_user_mapping_falls_through():
    conn = make_conn()
    hevy_db.save_mapping(conn, "79D0BB3A", 24, 3)
    hevy_db.mark_mapping_rejected(conn, "79D0BB3A")
    cat, sub, _ = lookup_exercise(conn, "Bench Press (Barbell)", "79D0BB3A")
    assert (cat, sub) == (0, 1)  # falls back to the template map


def test_name_table_hit_without_template_id():
    conn = make_conn()
    cat, sub, _ = lookup_exercise(conn, "Bench Press (Barbell)", None)
    assert (cat, sub) == (0, 1)


def test_miss_raises_mapping_miss():
    conn = make_conn()
    with pytest.raises(MappingMiss) as exc_info:
        lookup_exercise(conn, "Totally Made Up Movement", "NOTATEMPLATE")
    assert exc_info.value.template_id == "NOTATEMPLATE"
    assert exc_info.value.title == "Totally Made Up Movement"


def test_lookup_never_returns_unknown():
    conn = make_conn()
    # Sanity: nothing in the resolution chain may return the UNKNOWN sentinel
    with pytest.raises(MappingMiss):
        lookup_exercise(conn, "", None)
    assert UNKNOWN_CATEGORY == 65534


# -- suggest_mapping --------------------------------------------------------

def test_suggest_fuzzy_hit_on_typo():
    got = suggest_mapping("Bench Press (Barbel)", None)  # missing final 'l'
    assert got == (0, 1)


def test_suggest_muscle_group_fallback():
    template = {"primary_muscle_group": "quadriceps", "equipment_category": "barbell"}
    assert suggest_mapping("Custom Quad Blaster 3000", template) == (28, 0)


def test_suggest_none_when_no_signal():
    assert suggest_mapping("zzzzzz qqqq", None) is None
    assert suggest_mapping("zzzzzz qqqq", {"primary_muscle_group": "mystery"}) is None


# -- name tables ------------------------------------------------------------

def test_category_names_cover_fit_categories():
    assert CATEGORY_NAMES[0] == "BENCH_PRESS"
    assert CATEGORY_NAMES[28] == "SQUAT"
    assert CATEGORY_NAMES[65534] == "UNKNOWN"


def test_subcategory_names_resolve():
    assert SUBCATEGORY_NAMES[0][1] == "BARBELL_BENCH_PRESS"
    # unresolvable pair → absent (payload builder then sends null name)
    assert 999 not in SUBCATEGORY_NAMES.get(0, {})


def test_all_template_map_categories_have_names():
    from activsync.hevy_template_map import TEMPLATE_TO_GARMIN
    cats = {cat for cat, _sub in TEMPLATE_TO_GARMIN.values()}
    missing = cats - set(CATEGORY_NAMES)
    assert not missing, f"categories with no name: {missing}"
