//! Exercise lookups for Hevy workouts, as in the Python app's `hevy_mapper`
//! and `strava_exercise_mapper`, over tables exported from it by
//! `assets/export_exercise_tables.py`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::LazyLock;

use rusqlite::{Connection, OptionalExtension};
use serde::Deserialize;

use crate::db::DbResult;

pub const UNKNOWN_CATEGORY: i64 = 65534;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Raw {
    categories: Vec<(i64, String)>,
    subcategories: Vec<(i64, Vec<(i64, String)>)>,
    hevy_to_garmin: Vec<(String, (i64, i64))>,
    template_overrides: HashMap<String, (i64, i64)>,
    template_to_garmin: HashMap<String, (i64, i64)>,
    template_to_strava: HashMap<String, String>,
    strava_groups: Vec<(String, Vec<String>)>,
    strava_types: Vec<String>,
}

pub struct Tables {
    pub categories: BTreeMap<i64, String>,
    pub subcategories: BTreeMap<i64, BTreeMap<i64, String>>,
    hevy_to_garmin: HashMap<String, (i64, i64)>,
    normalized_names: Vec<(String, (i64, i64))>,
    template_overrides: HashMap<String, (i64, i64)>,
    template_to_garmin: HashMap<String, (i64, i64)>,
    template_to_strava: HashMap<String, String>,
    pub strava_groups: Vec<(String, Vec<String>)>,
    pub strava_types: HashSet<String>,
}

pub static TABLES: LazyLock<Tables> = LazyLock::new(|| {
    let raw: Raw = serde_json::from_str(include_str!("../assets/exercise_tables.json"))
        .expect("exercise_tables.json is valid");
    let mut normalized: Vec<(String, (i64, i64))> = Vec::new();
    for (name, pair) in raw
        .hevy_to_garmin
        .iter()
        .filter(|(_, pair)| pair.0 != UNKNOWN_CATEGORY)
    {
        let key = normalize(name);
        match normalized.iter_mut().find(|(existing, _)| *existing == key) {
            Some(entry) => entry.1 = *pair,
            None => normalized.push((key, *pair)),
        }
    }
    Tables {
        categories: raw.categories.into_iter().collect(),
        subcategories: raw
            .subcategories
            .into_iter()
            .map(|(c, names)| (c, names.into_iter().collect()))
            .collect(),
        hevy_to_garmin: raw.hevy_to_garmin.into_iter().collect(),
        normalized_names: normalized,
        template_overrides: raw.template_overrides,
        template_to_garmin: raw.template_to_garmin,
        template_to_strava: raw.template_to_strava,
        strava_groups: raw.strava_groups,
        strava_types: raw.strava_types.into_iter().collect(),
    }
});

impl Tables {
    pub fn subcategory_name(&self, category: i64, subcategory: i64) -> Option<&str> {
        self.subcategories
            .get(&category)?
            .get(&subcategory)
            .map(String::as_str)
    }

    pub fn is_valid_pair(&self, category: i64, subcategory: i64) -> bool {
        self.categories.contains_key(&category)
            && self.subcategory_name(category, subcategory).is_some()
    }
}

const ACRONYMS: [&str; 7] = ["BOSU", "EZ", "GHD", "HIIT", "KBS", "RDL", "TRX"];

/// A FIT enum identifier as a sentence-case label; the raw name is what
/// Garmin is sent.
pub fn garmin_exercise_label(enum_name: &str) -> String {
    let words: Vec<String> = enum_name
        .split('_')
        .map(|word| {
            if ACRONYMS.contains(&word) {
                word.to_string()
            } else {
                word.to_lowercase()
            }
        })
        .collect();
    let label = words.join(" ");
    let mut chars = label.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => label,
    }
}

pub fn strava_exercise_label(exercise_type: &str) -> String {
    match exercise_type.split_once("_GENERIC") {
        Some((base, _)) => format!("{} (generic)", garmin_exercise_label(base)),
        None => garmin_exercise_label(exercise_type),
    }
}

fn normalize(title: &str) -> String {
    let lower = title.to_lowercase();
    let kept: String = lower
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == ' ')
        .collect();
    kept.trim_matches(' ').to_string()
}

/// difflib's `SequenceMatcher(None, a, b).ratio()`. Without junk (b is
/// shorter than autojunk's 200 characters) the longest-match extension steps
/// never apply.
fn sequence_ratio(a: &[char], b: &[char]) -> f64 {
    let mut b2j: HashMap<char, Vec<usize>> = HashMap::new();
    for (j, c) in b.iter().enumerate() {
        b2j.entry(*c).or_default().push(j);
    }
    let longest = |alo: usize, ahi: usize, blo: usize, bhi: usize| {
        let (mut best_i, mut best_j, mut best_size) = (alo, blo, 0);
        let mut j2len: HashMap<usize, usize> = HashMap::new();
        for (i, c) in a.iter().enumerate().take(ahi).skip(alo) {
            let mut next: HashMap<usize, usize> = HashMap::new();
            for &j in b2j.get(c).map(Vec::as_slice).unwrap_or_default() {
                if j < blo {
                    continue;
                }
                if j >= bhi {
                    break;
                }
                let k = j
                    .checked_sub(1)
                    .and_then(|p| j2len.get(&p))
                    .copied()
                    .unwrap_or(0)
                    + 1;
                next.insert(j, k);
                if k > best_size {
                    (best_i, best_j, best_size) = (i + 1 - k, j + 1 - k, k);
                }
            }
            j2len = next;
        }
        (best_i, best_j, best_size)
    };
    let mut matches = 0;
    let mut queue = vec![(0, a.len(), 0, b.len())];
    while let Some((alo, ahi, blo, bhi)) = queue.pop() {
        let (i, j, k) = longest(alo, ahi, blo, bhi);
        if k > 0 {
            matches += k;
            if alo < i && blo < j {
                queue.push((alo, i, blo, j));
            }
            if i + k < ahi && j + k < bhi {
                queue.push((i + k, ahi, j + k, bhi));
            }
        }
    }
    let total = a.len() + b.len();
    if total == 0 {
        1.0
    } else {
        2.0 * matches as f64 / total as f64
    }
}

/// `difflib.get_close_matches(word, possibilities, n=1, cutoff=0.6)`.
fn closest<'a>(word: &str, possibilities: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    let b: Vec<char> = word.chars().collect();
    let mut best: Option<(f64, &str)> = None;
    for candidate in possibilities {
        let a: Vec<char> = candidate.chars().collect();
        let score = sequence_ratio(&a, &b);
        if score < 0.6 {
            continue;
        }
        let better = match best {
            None => true,
            Some((top, name)) => score > top || (score == top && candidate > name),
        };
        if better {
            best = Some((score, candidate));
        }
    }
    best.map(|(_, name)| name)
}

pub fn user_mapping(conn: &Connection, template_id: &str) -> DbResult<Option<(i64, i64)>> {
    conn.query_row(
        "SELECT category, subcategory FROM exercise_mappings WHERE exercise_template_id = ?1",
        [template_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
}

/// The built-in pair, ignoring saved mappings: what "Reset to standard"
/// would restore.
pub fn standard_mapping(title: &str, template_id: Option<&str>) -> Option<(i64, i64)> {
    let tables = &*TABLES;
    if let Some(id) = template_id.filter(|id| !id.is_empty()) {
        let pair = tables
            .template_overrides
            .get(id)
            .or_else(|| tables.template_to_garmin.get(id));
        if let Some(pair) = pair.filter(|p| p.0 != UNKNOWN_CATEGORY) {
            return Some(*pair);
        }
    }
    tables
        .hevy_to_garmin
        .get(title)
        .filter(|p| p.0 != UNKNOWN_CATEGORY)
        .copied()
}

/// Saved mapping, then the template tables, then the name table; `None` is
/// a miss that parks the workout for mapping.
pub fn lookup_exercise(
    conn: &Connection,
    title: &str,
    template_id: Option<&str>,
) -> DbResult<Option<(i64, i64)>> {
    if let Some(id) = template_id.filter(|id| !id.is_empty()) {
        if let Some(pair) = user_mapping(conn, id)?.filter(|p| p.0 != UNKNOWN_CATEGORY) {
            return Ok(Some(pair));
        }
    }
    Ok(standard_mapping(title, template_id))
}

fn muscle_group_fallback(muscle: &str) -> Option<i64> {
    Some(match muscle {
        "chest" => 0,
        "lats" | "upper_back" => 23,
        "biceps" => 7,
        "triceps" => 30,
        "quadriceps" => 28,
        "hamstrings" => 15,
        "glutes" => 10,
        "shoulders" => 24,
        "abdominals" => 5,
        "calves" => 1,
        "cardio" => 2,
        _ => return None,
    })
}

/// A starting point for the picker on an unmapped exercise: the closest
/// built-in name, else the template's muscle group.
pub fn suggest_mapping(title: &str, primary_muscle_group: Option<&str>) -> Option<(i64, i64)> {
    let normalized = normalize(title);
    if !normalized.is_empty() {
        let names = &TABLES.normalized_names;
        if let Some(name) = closest(&normalized, names.iter().map(|(n, _)| n.as_str())) {
            return names.iter().find(|(n, _)| n == name).map(|(_, pair)| *pair);
        }
    }
    muscle_group_fallback(primary_muscle_group.unwrap_or_default()).map(|category| (category, 0))
}

pub fn user_strava_exercise(conn: &Connection, template_id: &str) -> DbResult<Option<String>> {
    conn.query_row(
        "SELECT exercise_type FROM strava_exercise_mappings WHERE exercise_template_id = ?1",
        [template_id],
        |row| row.get(0),
    )
    .optional()
}

/// Strava's exercise without the owner's choice, with where it came from:
/// the template table, or the Garmin mapping's exercise or generic id.
pub fn default_strava_exercise(
    conn: &Connection,
    title: &str,
    template_id: Option<&str>,
) -> DbResult<Option<(String, &'static str)>> {
    let tables = &*TABLES;
    if let Some(standard) = template_id
        .filter(|id| !id.is_empty())
        .and_then(|id| tables.template_to_strava.get(id))
        .filter(|t| tables.strava_types.contains(*t))
    {
        return Ok(Some((standard.clone(), "standard")));
    }
    let Some((category, subcategory)) = lookup_exercise(conn, title, template_id)? else {
        return Ok(None);
    };
    if let Some(name) = tables
        .subcategory_name(category, subcategory)
        .filter(|n| tables.strava_types.contains(*n))
    {
        return Ok(Some((name.to_string(), "garmin")));
    }
    let generic = format!(
        "{}_GENERIC",
        tables
            .categories
            .get(&category)
            .map(String::as_str)
            .unwrap_or("None")
    );
    Ok(tables
        .strava_types
        .contains(&generic)
        .then_some((generic, "garmin")))
}

pub fn lookup_strava_exercise(
    conn: &Connection,
    title: &str,
    template_id: Option<&str>,
) -> DbResult<Option<(String, &'static str)>> {
    if let Some(id) = template_id.filter(|id| !id.is_empty()) {
        if let Some(user) =
            user_strava_exercise(conn, id)?.filter(|t| TABLES.strava_types.contains(t))
        {
            return Ok(Some((user, "user")));
        }
    }
    default_strava_exercise(conn, title, template_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratio_matches_difflib() {
        let ratio = |a: &str, b: &str| {
            sequence_ratio(
                &a.chars().collect::<Vec<_>>(),
                &b.chars().collect::<Vec<_>>(),
            )
        };
        assert_eq!(ratio("abcd", "bcde"), 0.75);
        assert_eq!(
            ratio("bench press barbell", "bench press dumbbell"),
            0.8717948717948718
        );
        assert_eq!(ratio("", ""), 1.0);
    }

    #[test]
    fn labels_match_python() {
        assert_eq!(garmin_exercise_label("BARBELL_RDL"), "Barbell RDL");
        assert_eq!(
            strava_exercise_label("BENCH_PRESS_GENERIC"),
            "Bench press (generic)"
        );
    }
}
