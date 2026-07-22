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
    lookup_standard_mapping,
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


def test_name_table_hit_without_template_id():
    conn = make_conn()
    cat, sub, _ = lookup_exercise(conn, "Bench Press (Barbell)", None)
    assert (cat, sub) == (0, 1)


def test_standard_mapping_ignores_user_override():
    conn = make_conn()
    hevy_db.save_mapping(conn, "79D0BB3A", 24, 3)

    assert lookup_exercise(conn, "Bench Press (Barbell)", "79D0BB3A")[:2] == (24, 3)
    assert lookup_standard_mapping("Bench Press (Barbell)", "79D0BB3A") == (0, 1)


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


def test_unknown_entries_in_tables_raise_mapping_miss():
    # Both ported tables contain UNKNOWN (65534) entries (e.g. neck
    # exercises). Those must behave as misses, not resolve.
    conn = make_conn()
    from activsync.hevy_mapper import HEVY_TO_GARMIN
    from activsync.hevy_template_map import TEMPLATE_TO_GARMIN

    unknown_names = [n for n, (c, _s) in HEVY_TO_GARMIN.items() if c == UNKNOWN_CATEGORY]
    unknown_templates = [t for t, (c, _s) in TEMPLATE_TO_GARMIN.items()
                         if c == UNKNOWN_CATEGORY]
    assert unknown_names, "expected UNKNOWN entries in the name table"
    with pytest.raises(MappingMiss):
        lookup_exercise(conn, unknown_names[0], None)
    if unknown_templates:
        with pytest.raises(MappingMiss):
            lookup_exercise(conn, "Whatever Title", unknown_templates[0])


def test_unknown_user_mapping_is_rejected():
    # A user mapping row carrying the sentinel must not resolve either.
    conn = make_conn()
    hevy_db.save_mapping(conn, "TPLX", UNKNOWN_CATEGORY, 0)
    with pytest.raises(MappingMiss):
        lookup_exercise(conn, "Some Exercise", "TPLX")


def test_suggest_mapping_never_suggests_unknown():
    from activsync.hevy_mapper import HEVY_TO_GARMIN

    unknown_names = [n for n, (c, _s) in HEVY_TO_GARMIN.items() if c == UNKNOWN_CATEGORY]
    for name in unknown_names:
        got = suggest_mapping(name, None)
        assert got is None or got[0] != UNKNOWN_CATEGORY


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
