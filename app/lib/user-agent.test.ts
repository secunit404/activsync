import { expect, test } from "vitest";

import { describeUserAgent } from "@/lib/user-agent";

const cases: Array<[string, string, string]> = [
  [
    "Firefox on macOS",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:131.0) Gecko/20100101 Firefox/131.0",
    "Firefox on macOS",
  ],
  [
    "Chrome on Windows",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36",
    "Chrome on Windows",
  ],
  [
    "Edge on Windows",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36 Edg/130.0.0.0",
    "Edge on Windows",
  ],
  [
    "Safari on iOS",
    "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1",
    "Safari on iOS",
  ],
  [
    "Chrome on iOS",
    "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) CriOS/130.0.0.0 Mobile/15E148 Safari/604.1",
    "Chrome on iOS",
  ],
  [
    "Chrome on Android",
    "Mozilla/5.0 (Linux; Android 14; Pixel 7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Mobile Safari/537.36",
    "Chrome on Android",
  ],
  [
    "Safari on macOS",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15",
    "Safari on macOS",
  ],
  [
    "Firefox on Linux",
    "Mozilla/5.0 (X11; Linux x86_64; rv:131.0) Gecko/20100101 Firefox/131.0",
    "Firefox on Linux",
  ],
  ["a bare browser name", "Firefox/131.0", "Firefox"],
];

test.each(cases)("describes %s", (_name, userAgent, expected) => {
  expect(describeUserAgent(userAgent)).toBe(expected);
});

test.each([[""], ["curl/8.4.0"], ["python-requests/2.31"]])(
  "returns null when %j has no recognisable browser",
  (userAgent) => {
    expect(describeUserAgent(userAgent)).toBeNull();
  },
);
