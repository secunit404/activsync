import { expect, test } from "vitest";

import {
  renderDescriptionPreview,
  renderTitlePreview,
} from "./hevy-description";

test("renders placeholders with representative plain-text workout data", () => {
  const preview = renderDescriptionPreview("{duration}\n\n{exercises}\n\n{marker}");

  expect(preview.error).toBeNull();
  expect(preview.text).toContain("⏱️ 68 min");
  expect(preview.text).toContain("• Chest Fly (Machine)");
  expect(preview.text).toContain("— synced by activsync");
});

test("renders clean_title only in the separate title preview", () => {
  expect(renderTitlePreview("{clean_title}")).toEqual({
    text: "Afternoon workout",
    error: null,
  });
  expect(renderDescriptionPreview("{clean_title}").error).toMatch(
    /Unknown placeholder/,
  );
});

test("keeps markdown and HTML literal because destination descriptions are plain text", () => {
  const preview = renderDescriptionPreview("**Summary**\n<em>{duration}</em>");

  expect(preview.text).toContain("**Summary**");
  expect(preview.text).toContain("<em>⏱️ 68 min</em>");
});

test("reports unknown placeholders and supports escaped literal braces", () => {
  expect(renderDescriptionPreview("{unknown}").error).toMatch(/Unknown placeholder/);
  expect(renderDescriptionPreview("{{literal}} {duration}")).toEqual({
    text: "{literal} ⏱️ 68 min",
    error: null,
  });
});
