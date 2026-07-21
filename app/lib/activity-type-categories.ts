/**
 * Garmin's `fetch_activity_types` (src/activsync/garmin_client.py) returns a
 * flat, alphabetically-sorted `{type_key, label}` list — there is no
 * category/parent field anywhere in the taxonomy it reads from, so the
 * design handoff's category headers ("RUNNING", "CYCLING", …) can't come
 * from the API. This module derives a reasonable grouping client-side from
 * the type key alone, purely for display. It is a UI convenience, not a
 * source of truth: unmatched or future Garmin type keys fall into "Other"
 * rather than being dropped or throwing.
 */

// A handful of type keys contain a substring that would otherwise sort them
// into the wrong bucket under the keyword rules below (e.g. "stair_climbing"
// contains "climb", "sky_diving" contains "diving"). Checked before the
// keyword rules.
const CATEGORY_OVERRIDES: Record<string, string> = {
  stair_climbing: "Strength & fitness",
  indoor_rowing: "Strength & fitness",
  sky_diving: "Motor sports",
};

// Ordered keyword rules: first match wins. Order matters where keywords
// could otherwise collide (e.g. "transition" is checked before "run"/"bike"
// so the biketoruntransition_v2-style keys land in Multisport, not Running).
const CATEGORY_RULES: Array<{ category: string; keywords: string[] }> = [
  { category: "Multisport", keywords: ["transition", "multi_sport"] },
  { category: "Running", keywords: ["run"] },
  { category: "Cycling", keywords: ["cycl", "biking", "bike", "mtb", "bmx", "ride"] },
  { category: "Swimming", keywords: ["swim"] },
  { category: "Walking", keywords: ["walk"] },
  { category: "Hiking", keywords: ["hik", "mountaineering", "rucking"] },
  { category: "Climbing", keywords: ["climb", "bouldering"] },
  {
    category: "Winter sports",
    keywords: ["ski", "snowboard", "snow_shoe", "snowmobiling", "skating_ws", "winter"],
  },
  {
    category: "Water sports",
    keywords: [
      "paddl",
      "kayak",
      "row",
      "surf",
      "sail",
      "boating",
      "wake",
      "waterski",
      "water_tubing",
      "kiteboard",
      "whitewater",
      "grinding",
      "water",
    ],
  },
  { category: "Diving", keywords: ["diving", "apnea", "snorkel"] },
  {
    category: "Racquet sports",
    keywords: [
      "tennis",
      "squash",
      "racquetball",
      "badminton",
      "pickleball",
      "paddelball",
      "racket",
      "platform_tennis",
    ],
  },
  {
    category: "Team sports",
    keywords: [
      "soccer",
      "basketball",
      "baseball",
      "softball",
      "football",
      "hockey",
      "rugby",
      "lacrosse",
      "cricket",
      "volleyball",
      "ultimate_disc",
      "team_sports",
    ],
  },
  {
    category: "Strength & fitness",
    keywords: [
      "strength",
      "hiit",
      "pilates",
      "yoga",
      "elliptical",
      "jump_rope",
      "cardio",
      "fitness_equipment",
      "mobility",
      "breathwork",
      "meditation",
      "dance",
      "boxing",
      "martial_arts",
    ],
  },
  {
    category: "Motor sports",
    keywords: [
      "racing",
      "motorcycl",
      "motocross",
      "atv",
      "driving",
      "drone",
      "gliding",
      "flying",
    ],
  },
  { category: "Golf", keywords: ["golf"] },
];

const OTHER_CATEGORY = "Other";

/** Derives a display category from a Garmin activity type key. Pure and total — every key resolves to some category, defaulting to "Other". */
export function categorizeActivityType(typeKey: string): string {
  const override = CATEGORY_OVERRIDES[typeKey];
  if (override) {
    return override;
  }
  for (const rule of CATEGORY_RULES) {
    if (rule.keywords.some((keyword) => typeKey.includes(keyword))) {
      return rule.category;
    }
  }
  return OTHER_CATEGORY;
}

export type ActivityTypeOption = {
  typeKey: string;
  label: string;
  autosync: boolean;
};

/**
 * Groups activity types by derived category, preserving each group's
 * incoming (already alphabetical-by-label) order and sorting the groups
 * themselves alphabetically by category name for a stable, predictable
 * render order.
 */
export function groupActivityTypesByCategory<T extends { typeKey: string }>(
  items: readonly T[],
): Array<{ category: string; items: T[] }> {
  const buckets = new Map<string, T[]>();
  for (const item of items) {
    const category = categorizeActivityType(item.typeKey);
    const bucket = buckets.get(category);
    if (bucket) {
      bucket.push(item);
    } else {
      buckets.set(category, [item]);
    }
  }
  return [...buckets.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([category, categoryItems]) => ({ category, items: categoryItems }));
}
