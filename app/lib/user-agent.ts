const BROWSERS: ReadonlyArray<readonly [RegExp, string]> = [
  [/\bEdg(?:e|A|iOS)?\//, "Edge"],
  [/\bOPR\/|\bOpera\b/, "Opera"],
  [/\bFirefox\/|\bFxiOS\//, "Firefox"],
  [/\bChrome\/|\bCriOS\//, "Chrome"],
  [/\bSafari\//, "Safari"],
];

const SYSTEMS: ReadonlyArray<readonly [RegExp, string]> = [
  [/\biPhone\b|\biPod\b/, "iOS"],
  [/\biPad\b/, "iPadOS"],
  [/\bAndroid\b/, "Android"],
  [/\bWindows\b/, "Windows"],
  [/\bCrOS\b/, "ChromeOS"],
  [/\bMac OS X\b|\bMacintosh\b/, "macOS"],
  [/\bLinux\b|\bX11\b/, "Linux"],
];

function firstMatch(table: ReadonlyArray<readonly [RegExp, string]>, userAgent: string) {
  return table.find(([pattern]) => pattern.test(userAgent))?.[1] ?? null;
}

/** "Firefox on macOS" from a stored user agent, or null when no browser is recognisable. */
export function describeUserAgent(userAgent: string): string | null {
  const browser = firstMatch(BROWSERS, userAgent);
  if (!browser) return null;
  const system = firstMatch(SYSTEMS, userAgent);
  return system ? `${browser} on ${system}` : browser;
}
