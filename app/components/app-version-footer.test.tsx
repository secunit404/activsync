import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { AppVersionFooter } from "./app-version-footer";

const update = {
  latest: null,
  available: false,
  repoUrl: "https://github.com/secunit404/activsync",
  releaseUrl: "https://github.com/secunit404/activsync/releases/latest",
};

describe("AppVersionFooter", () => {
  it("shows a release as a version", () => {
    render(<AppVersionFooter version="1.2.4" release update={update} />);

    expect(screen.getByText("v1.2.4")).toBeInTheDocument();
  });

  it("collapses a branch build to dev and keeps the full tag in the tooltip", () => {
    render(
      <AppVersionFooter
        version="dev-feat-hevy-react-frontend"
        release={false}
        update={update}
      />,
    );

    expect(screen.getByText("dev")).toHaveAttribute(
      "title",
      "ActivSync dev-feat-hevy-react-frontend",
    );
  });
});
