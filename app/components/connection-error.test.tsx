import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";

import { ConnectionError } from "./connection-error";

test("is an alert that names the failure and offers a retry", async () => {
  const onRetry = vi.fn();
  render(<ConnectionError onRetry={onRetry} />);

  const alert = screen.getByRole("alert");
  expect(alert).toHaveTextContent("Couldn’t reach ActivSync");
  expect(alert).toHaveTextContent("didn’t respond");

  await userEvent.click(screen.getByRole("button", { name: "Try again" }));
  expect(onRetry).toHaveBeenCalledOnce();
});

test("without a status, blames unreachability rather than the backend", () => {
  render(<ConnectionError onRetry={vi.fn()} />);
  expect(screen.getByRole("alert")).toHaveTextContent(
    "The server didn’t respond. Check that the container is running.",
  );
});

// A response that reached the backend (a 500, say) is not the same failure
// as the container being down — misdirecting the user to check the
// container when the real cause is a backend error is exactly what this
// guards against (see the checkpoint review's IMPORTANT 6).
test("with a status, reports the error responded rather than claiming it didn't", () => {
  render(<ConnectionError onRetry={vi.fn()} status={500} detail="Internal Server Error" />);

  const alert = screen.getByRole("alert");
  expect(alert).toHaveTextContent("The server responded with an error (500).");
  expect(alert).not.toHaveTextContent("didn’t respond");
  expect(alert).toHaveTextContent("Internal Server Error");
});
