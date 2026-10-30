import { expect, test } from "vitest";

import { meta as activitiesMeta } from "./activities";
import { meta as hevyMeta } from "./hevy";
import { meta as hevyMappingsMeta } from "./hevy-mappings";
import { meta as loginMeta } from "./login";
import { meta as settingsMeta } from "./settings";
import { meta as setupMeta } from "./setup";

test.each([
  ["Activities", activitiesMeta],
  ["Hevy", hevyMeta],
  ["Exercise mapping", hevyMappingsMeta],
  ["Settings", settingsMeta],
  ["Sign in", loginMeta],
  ["Setup", setupMeta],
])("the %s tab title names the page, then ActivSync", (page, meta) => {
  expect(meta()).toContainEqual({ title: `${page} - ActivSync` });
});
