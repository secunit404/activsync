import { ArrowDownUpIcon } from "lucide-react";
import { useSearchParams } from "react-router";

import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select";
import type { SortOrder } from "@/lib/api";

/**
 * Reads/writes the `sort` search param directly (self-contained, same
 * convention as `ActivityFilterPills` and `ActivitiesPagination`) rather
 * than taking a callback — the Activities route re-derives its query from
 * the same URL on the next render.
 *
 * `newest` and `oldest` are the only orders the backend supports
 * (`api_routes.py`'s `SortOrder`), so this offers exactly those two.
 */
export function ActivitySortSelect() {
  const [searchParams, setSearchParams] = useSearchParams();
  const value: SortOrder = searchParams.get("sort") === "oldest" ? "oldest" : "newest";

  function selectSort(next: string) {
    const params = new URLSearchParams(searchParams);
    if (next === "oldest") {
      params.set("sort", "oldest");
    } else {
      // `newest` is the backend default — omitting it keeps shared URLs
      // clean rather than pinning the default into every link.
      params.delete("sort");
    }
    // A re-sorted list makes the current page number meaningless.
    params.delete("page");
    setSearchParams(params);
  }

  const next: SortOrder = value === "newest" ? "oldest" : "newest";

  // Below `sm:` this is a toggle, not a picker: with only two orders, a menu
  // buys nothing, and the shrunk-to-44px `<select>` it replaces opened its
  // native popup detached from the control. Styled like the filter pills it
  // shares the row with (same border, same muted icon).
  return (
    <>
      <button
        type="button"
        aria-label={`Sort ${next} first`}
        onClick={() => selectSort(next)}
        className="flex size-11 shrink-0 items-center justify-center rounded-xl border border-border text-muted-foreground transition-colors hover:text-foreground sm:hidden"
      >
        <ArrowDownUpIcon aria-hidden="true" className="size-4" />
      </button>
      <NativeSelect
        aria-label="Sort activities"
        className="hidden w-auto shrink-0 sm:block"
        value={value}
        onChange={(event) => selectSort(event.target.value)}
      >
        <NativeSelectOption value="newest">Newest first</NativeSelectOption>
        <NativeSelectOption value="oldest">Oldest first</NativeSelectOption>
      </NativeSelect>
    </>
  );
}
