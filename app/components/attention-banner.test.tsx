import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";

import { AttentionBanner } from "./attention-banner";

test("attention banner names the disconnected service and offers reconnect", async () => {
  const onReconnect = vi.fn();
  render(<AttentionBanner services={["strava"]} onReconnect={onReconnect} />);
  expect(screen.getByRole("status")).toHaveTextContent(/Strava/);
  await userEvent.click(screen.getByRole("button", { name: "Reconnect" }));
  expect(onReconnect).toHaveBeenCalledWith("strava");
});

test("names Garmin when Garmin is the disconnected service", () => {
  render(<AttentionBanner services={["garmin"]} onReconnect={vi.fn()} />);
  expect(screen.getByRole("status")).toHaveTextContent(/Garmin/);
});

test("both services broken make one banner with a reconnect action for each", async () => {
  const onReconnect = vi.fn();
  render(<AttentionBanner services={["garmin", "strava"]} onReconnect={onReconnect} />);

  expect(screen.getAllByRole("status")).toHaveLength(1);
  expect(screen.getByText("Reconnect Garmin and Strava")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Reconnect Garmin" }));
  await userEvent.click(screen.getByRole("button", { name: "Reconnect Strava" }));
  expect(onReconnect.mock.calls).toEqual([["garmin"], ["strava"]]);
});

test("an outage is standing state, never an interrupting alert", () => {
  render(<AttentionBanner services={["strava"]} onReconnect={vi.fn()} />);
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
});

test("nothing broken renders nothing", () => {
  const { container } = render(<AttentionBanner services={[]} onReconnect={vi.fn()} />);
  expect(container).toBeEmptyDOMElement();
});
