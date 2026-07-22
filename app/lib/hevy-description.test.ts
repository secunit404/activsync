import { expect, test } from "vitest";

import { renderDescriptionPreview } from "./hevy-description";

test("renders placeholders with representative plain-text workout data", () => {
  const preview = renderDescriptionPreview("{title}\n\n{exercises}\n\n{marker}");

  expect(preview.error).toBeNull();
  expect(preview.text).toContain("Afternoon workout 💪");
  expect(preview.text).toContain("• Chest Fly (Machine)");
  expect(preview.text).toContain("— synced by activsync");
});

test("renders clean_title without the emoji from the Hevy title", () => {
  expect(renderDescriptionPreview("{clean_title}")).toEqual({
    text: "Afternoon workout",
    error: null,
  });
});

test("keeps markdown and HTML literal because destination descriptions are plain text", () => {
  const preview = renderDescriptionPreview("**{title}**\n<em>{duration}</em>");

  expect(preview.text).toContain("**Afternoon workout 💪**");
  expect(preview.text).toContain("<em>⏱️ 68 min</em>");
});

test("reports unknown placeholders and supports escaped literal braces", () => {
  expect(renderDescriptionPreview("{unknown}").error).toMatch(/Unknown placeholder/);
  expect(renderDescriptionPreview("{{literal}} {title}")).toEqual({
    text: "{literal} Afternoon workout 💪",
    error: null,
  });
});
