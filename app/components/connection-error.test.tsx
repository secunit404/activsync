import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";

import { ConnectionError } from "./connection-error";

test("is an alert that names the failure and offers a retry", async () => {
  const onRetry = vi.fn();
  render(<ConnectionError onRetry={onRetry} />);

  const alert = screen.getByRole("alert");
  expect(alert).toHaveTextContent("Couldn’t reach ActivSync");

  await userEvent.click(screen.getByRole("button", { name: "Try again" }));
  expect(onRetry).toHaveBeenCalledOnce();
});
