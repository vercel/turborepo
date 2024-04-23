import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, test } from "vitest";
import { Card } from "./card";

describe.concurrent("card", () => {
	afterEach(cleanup);

	test("check if h1 heading exists", ({ expect }) => {
		render(<Card />);
		expect(screen.getByTestId("card-h1").textContent).toBe("card");
	});
});
