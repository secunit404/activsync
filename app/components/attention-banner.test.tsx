import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";

import { AttentionBanner } from "./attention-banner";

test("attention banner names the disconnected service and offers reconnect", async () => {
  const onReconnect = vi.fn();
  render(<AttentionBanner service="strava" onReconnect={onReconnect} />);
  expect(screen.getByRole("alert")).toHaveTextContent(/Strava/);
  await userEvent.click(screen.getByRole("button", { name: /reconnect/i }));
  expect(onReconnect).toHaveBeenCalled();
});

test("names Garmin when Garmin is the disconnected service", () => {
  render(<AttentionBanner service="garmin" onReconnect={vi.fn()} />);
  expect(screen.getByRole("alert")).toHaveTextContent(/Garmin/);
});
