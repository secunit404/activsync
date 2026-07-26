const SAMPLE_TITLE_VALUES: Record<string, string> = {
  title: "Afternoon workout 💪",
  clean_title: "Afternoon workout",
};

const SAMPLE_DESCRIPTION_VALUES: Record<string, string> = {
  duration: "⏱️ 68 min",
  calories: "🔥 386 kcal",
  avg_hr: "❤️ avg 104 bpm",
  exercises: [
    "• Chest Fly (Machine): 3 sets · 57.5kg × 9",
    "• Lat Pulldown (Cable): 3 sets · 50.0kg × 9",
    "• Bulgarian Split Squat (Barbell): 3 sets · 25.0kg × 8",
  ].join("\n"),
  marker: "— synced by activsync",
};

const OPEN_BRACE_TOKEN = "\u0000open-brace\u0000";
const CLOSE_BRACE_TOKEN = "\u0000close-brace\u0000";

export type DescriptionPreview = { text: string; error: string | null };

function renderTemplatePreview(
  template: string,
  sampleValues: Record<string, string>,
): DescriptionPreview {
  let protectedTemplate = template
    .replaceAll("{{", OPEN_BRACE_TOKEN)
    .replaceAll("}}", CLOSE_BRACE_TOKEN);
  let error: string | null = null;

  protectedTemplate = protectedTemplate.replace(/\{([^{}]+)\}/g, (match, name) => {
    if (!(name in sampleValues)) {
      error ??= `Unknown placeholder: {${name}}`;
      return match;
    }
    return sampleValues[name];
  });
  if (error === null && /[{}]/.test(protectedTemplate)) {
    error = "A brace is unmatched. Use {{ and }} for literal braces.";
  }

  const rendered = protectedTemplate
    .replaceAll(OPEN_BRACE_TOKEN, "{")
    .replaceAll(CLOSE_BRACE_TOKEN, "}");
  const cleaned: string[] = [];
  for (const rawLine of rendered.split("\n")) {
    const line = rawLine.trimEnd();
    if (line || (cleaned.length > 0 && cleaned.at(-1) !== "")) {
      cleaned.push(line);
    }
  }
  while (cleaned.at(-1) === "") cleaned.pop();
  return { text: cleaned.join("\n"), error };
}

/** Render the same placeholder shape as the Python formatter, using sample
 * values only. This stays synchronous while the user types; Settings save is
 * still the authoritative server-side validation. */
export function renderDescriptionPreview(template: string): DescriptionPreview {
  return renderTemplatePreview(template, SAMPLE_DESCRIPTION_VALUES);
}

export function renderTitlePreview(template: string): DescriptionPreview {
  return renderTemplatePreview(template, SAMPLE_TITLE_VALUES);
}
