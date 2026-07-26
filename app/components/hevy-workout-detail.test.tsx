import { render, screen } from "@testing-library/react";
import { expect, test } from "vitest";

import { HevyWorkoutDetailContent } from "./hevy-workout-detail";
import type { HevyWorkoutDetail, HevyWorkoutSetDetail } from "@/lib/api";

function set(number: number, over: Partial<HevyWorkoutSetDetail> = {}): HevyWorkoutSetDetail {
  return {
    number,
    setType: "normal",
    weightKg: null,
    reps: null,
    distanceMeters: null,
    durationSeconds: null,
    customMetric: null,
    rpe: null,
    ...over,
  };
}

function detail(sets: HevyWorkoutSetDetail[], title = "Cable Fly"): HevyWorkoutDetail {
  return {
    hevyId: "w1",
    title: "Afternoon workout",
    startTime: "2026-07-21T17:44:00Z",
    endTime: "2026-07-21T18:43:00Z",
    notes: null,
    descriptionPreview: "preview",
    exercises: [{ templateId: "t1", title, notes: null, sets }],
  };
}

test("an exercise is collapsed by default so the sheet stays scannable", () => {
  render(<HevyWorkoutDetailContent detail={detail([set(1, { reps: 8 })])} />);
  // The set table lives inside the closed <details>, so it is present in the
  // DOM but not exposed — which is what keeps the sheet short.
  expect(screen.getByRole("group")).not.toHaveAttribute("open");
});

test("a collapsed exercise still reports one value when every set matches", () => {
  render(
    <HevyWorkoutDetailContent
      detail={detail([
        set(1, { weightKg: 70, reps: 8 }),
        set(2, { weightKg: 70, reps: 8 }),
      ])}
    />,
  );
  expect(screen.getByText("2 sets · 70 kg · 8 reps")).toBeInTheDocument();
});

test("a collapsed exercise reports a range when the sets differ", () => {
  render(
    <HevyWorkoutDetailContent
      detail={detail([
        set(1, { weightKg: 42, reps: 8 }),
        set(2, { weightKg: 46, reps: 6 }),
      ])}
    />,
  );
  expect(screen.getByText("2 sets · 42–46 kg · 6–8 reps")).toBeInTheDocument();
});

// Integers on purpose: `formatNumber` is locale-aware, so a fractional weight
// would assert "42,5" or "42.5" depending on where the suite runs.
// A bodyweight hold records neither weight nor reps — the summary must name
// what was recorded rather than inventing a dimension or rendering "null".
test("only dimensions the exercise actually recorded appear", () => {
  render(
    <HevyWorkoutDetailContent detail={detail([set(1, { durationSeconds: 90 })], "Plank")} />,
  );
  expect(screen.getByText("1 set · 1m 30s")).toBeInTheDocument();
});

test("falls back to the set count alone when nothing else was recorded", () => {
  render(<HevyWorkoutDetailContent detail={detail([set(1), set(2), set(3)])} />);
  expect(screen.getByText("3 sets")).toBeInTheDocument();
});
