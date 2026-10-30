/**
 * Focus targets on the Activities page, found by data attribute rather than
 * passed refs: the detail overlay is a separate route and the bulk bar a
 * sibling, and neither owns the rows. The table and the phone cards both
 * render every row, one of them `display:none`, so each lookup takes the
 * first match that is actually rendered.
 */
export const ACTIVITIES_HEADING_ID = "activities-heading";

function isRendered(element: HTMLElement): boolean {
  // jsdom has no layout and no checkVisibility, so tests take the first match.
  return typeof element.checkVisibility === "function" ? element.checkVisibility() : true;
}

function firstRendered(selector: string): HTMLElement | null {
  const matches = document.querySelectorAll<HTMLElement>(selector);
  return Array.from(matches).find(isRendered) ?? null;
}

function activityLink(id: number): HTMLElement | null {
  return firstRendered(`[data-activity-link="${id}"]`);
}

function listHeading(): HTMLElement | null {
  return document.getElementById(ACTIVITIES_HEADING_ID);
}

/**
 * Where focus goes when the detail overlay for `id` closes: its own row,
 * else the row that took its place (the next one, then the previous one,
 * in `order` — the list as it was while the activity was still in it),
 * else the page heading. Publish and Exclude can move an activity out of
 * the current filter while its detail is open.
 */
export function activityReturnTarget(id: number, order: readonly number[]): HTMLElement | null {
  const own = activityLink(id);
  if (own) {
    return own;
  }
  const index = order.indexOf(id);
  if (index !== -1) {
    const after = order.slice(index + 1);
    const before = order.slice(0, index).reverse();
    for (const neighbour of [...after, ...before]) {
      const link = activityLink(neighbour);
      if (link) {
        return link;
      }
    }
  }
  return listHeading();
}

/** Somewhere keyboard focus can land when the bulk bar it was in goes away. */
export function selectionFallbackTarget(): HTMLElement | null {
  return firstRendered("[data-activity-checkbox]") ?? listHeading();
}
