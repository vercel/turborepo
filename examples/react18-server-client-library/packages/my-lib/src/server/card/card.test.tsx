import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, test } from "vitest";
import { Card } from "./card";

describe("card", () => {
  const className = "test";
  const href = "test-href";
  const title = "test-title";
  const someDesc = "this is a test card";
  beforeEach(() => {
    render(
      <Card className={className} href={href} title={title}>
        {someDesc}
      </Card>,
    );
  });
  afterEach(cleanup);

  test("test className", ({ expect }) => {
    expect(screen.getByTestId("card").className).toBe(className);
  });

  test("test href", ({ expect }) => {
    expect(screen.getByTestId("card").getAttribute("href")).toBe(href);
  });

  test("test title", ({ expect }) => {
    expect(
      screen.getByTestId("card").getElementsByTagName("h2")[0]?.textContent,
    ).toBe(title + " ->");
  });

  test("test description", ({ expect }) => {
    expect(
      screen.getByTestId("card").getElementsByTagName("p")[0]?.textContent,
    ).toBe(someDesc);
  });
});
