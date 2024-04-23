import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, test } from "vitest";
import { Button } from "./button";

describe("button", () => {
  afterEach(cleanup);

  test("check ClassName and children", ({ expect }) => {
    render(
      <Button className="btn" appName="my-app">
        Click me
      </Button>,
    );
    expect(screen.getByTestId("button").textContent).toBe("Click me");
    expect(screen.getByTestId("button").className).toBe("btn");
  });
});
