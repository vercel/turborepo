// import * as Got from "got";
import { test, mock, describe } from "node:test";
import assert from "node:assert";
import got from "got";
import { isUrlOk } from "../dist/examples.js";

describe("isUrlOk", () => {
  test("returns true if url returns 200", async (t) => {
    const mockGot = t.mock.fn(got.head, () => {
      return { statusCode: 200 };
    });

    const url = "https://github.com/vercel/turbo/";
    const result = await isUrlOk(url);
    assert.strictEqual(result, true);

    assert.strictEqual(mockGot.mock.calls.length, 1);
    assert.strictEqual(mockGot.mock.calls[0].arguments, url);

    mock.reset();
  });
});
