import { useSearchParams } from "react-router";

import {
  Pagination,
  PaginationContent,
  PaginationItem,
  PaginationNext,
  PaginationPrevious,
} from "@/components/ui/pagination";

type ActivitiesPaginationProps = {
  page: number;
  pageCount: number;
};

/**
 * Self-contained like `ActivityFilterPills` (Task 7): reads/writes the
 * `page` URL param directly rather than taking an `onPageChange` callback,
 * so `ActivitiesView` doesn't need to own paging state. Ported from the
 * deleted `home.test.tsx`'s "reports next-page navigation" case, adapted to
 * that URL-param convention.
 */
export function ActivitiesPagination({ page, pageCount }: ActivitiesPaginationProps) {
  const [searchParams, setSearchParams] = useSearchParams();

  if (pageCount <= 1) {
    return null;
  }

  const goToPage = (nextPage: number) => {
    const next = new URLSearchParams(searchParams);
    if (nextPage <= 1) {
      next.delete("page");
    } else {
      next.set("page", String(nextPage));
    }
    setSearchParams(next);
  };

  return (
    <div className="flex items-center justify-end gap-4">
      <span className="font-mono text-xs text-muted-foreground">
        Page {page} of {pageCount}
      </span>
      <Pagination className="mx-0 w-auto">
        <PaginationContent>
          {page > 1 ? (
            <PaginationItem>
              <PaginationPrevious
                href="#"
                onClick={(event) => {
                  event.preventDefault();
                  goToPage(page - 1);
                }}
              />
            </PaginationItem>
          ) : null}
          {page < pageCount ? (
            <PaginationItem>
              <PaginationNext
                href="#"
                onClick={(event) => {
                  event.preventDefault();
                  goToPage(page + 1);
                }}
              />
            </PaginationItem>
          ) : null}
        </PaginationContent>
      </Pagination>
    </div>
  );
}
