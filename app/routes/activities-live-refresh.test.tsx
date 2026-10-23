import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createMemoryRouter, Outlet, RouterProvider } from "react-router";
import { expect, test, vi } from "vitest";

import Activities from "./activities";

const { useLiveRefresh } = vi.hoisted(() => ({ useLiveRefresh: vi.fn() }));

vi.mock("@/hooks/use-live-refresh", () => ({ useLiveRefresh }));

vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  const pending = () => new Promise<never>(() => {});
  return { ...actual, getAppState: pending, getActivities: pending };
});

test("the activities page leaves the live-refresh stream to the app root", async () => {
  const router = createMemoryRouter(
    [
      {
        path: "/",
        element: <Outlet context={() => {}} />,
        children: [{ index: true, element: <Activities /> }],
      },
    ],
    { initialEntries: ["/"] },
  );

  render(
    <QueryClientProvider client={new QueryClient()}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
  );

  await screen.findByTestId("activities-skeleton");
  expect(useLiveRefresh).not.toHaveBeenCalled();
});
