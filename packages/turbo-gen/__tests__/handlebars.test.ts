import { createRequire } from "node:module";
import { describe, expect, it } from "@jest/globals";
import Handlebars from "handlebars";

// Bypass Jest's node-plop mock to check its real transitive compiler as well
// as @turbo/gen's direct dependency.
const plopRequire = createRequire(
  createRequire(__filename).resolve("node-plop")
);
const plopHandlebars = plopRequire("handlebars") as typeof Handlebars;

function maliciousAst(handlebars: typeof Handlebars) {
  // Simulate an untrusted JSON AST with the CVE-2026-106446 blockParams payload.
  // oxlint-disable-next-line unicorn/prefer-structured-clone -- Exercise JSON-deserialized input.
  const ast = JSON.parse(
    JSON.stringify(handlebars.parse("{{#if ok}}safe{{/if}}"))
  );
  ast.body[0].program.blockParams = {
    length: "(()=>{throw new Error('AST_INJECTION_EXECUTED')})()"
  };
  return ast;
}

describe.each([
  ["@turbo/gen", Handlebars],
  ["node-plop", plopHandlebars]
])("%s Handlebars compiler", (_name, handlebars) => {
  it("renders and precompiles normal template strings", () => {
    const template = "Hello {{name}}!";
    expect(handlebars.compile(template)({ name: "Turborepo" })).toBe(
      "Hello Turborepo!"
    );
    expect(handlebars.precompile(template)).toEqual(expect.any(String));
  });

  it("rejects a malformed AST before executing injected JavaScript", () => {
    // compile is lazy, so render the result to trigger AST validation.
    expect(() =>
      handlebars.compile(maliciousAst(handlebars))({ ok: true })
    ).toThrow(/Invalid AST: Program blockParams must be an array/);
  });

  it("rejects a malformed AST instead of emitting injected JavaScript", () => {
    expect(() => handlebars.precompile(maliciousAst(handlebars))).toThrow(
      /Invalid AST: Program blockParams must be an array/
    );
  });
});
