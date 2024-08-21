#!/usr/bin/env node

import { writeFileSync } from "node:fs";
import { join } from "node:path";
import {
  DEFAULT_CONFIG,
  SchemaGenerator,
  createFormatter,
  createParser,
  createProgram,
  ts, // use the reexported TypeScript to avoid version conflicts
  type CompletedConfig,
} from "ts-json-schema-generator";

const __dirname = new URL(".", import.meta.url).pathname;
const packageRoot = join(__dirname, "..", "src");

/**
 * Unfortunately, we find ourselves in a world where TSDoc and TypeDoc use `@defaultValue`, expecting backticks around the value, while JSON Schema uses `default` without backticks.
 *
 * This function replaces `@defaultValue` with `@default` and removes backticks from the value.
 *
 * Needless to say, this is something that's pretty hacky to do, but ts-json-schema-generator doesn't provide a way to customize this behavior, so modifying the file with the TypeScript API (i.e. before it gets to the generator) is our only option.
 */
const replaceJSDoc = (
  node: ts.Node,
  context: ts.TransformationContext
): ts.Node => {
  if ("jsDoc" in node && Array.isArray(node.jsDoc)) {
    node.jsDoc.forEach((jsDoc: ts.Node) => {
      if (ts.isJSDoc(jsDoc)) {
        if (jsDoc.tags !== undefined && jsDoc.tags.length > 0) {
          jsDoc.tags.forEach((tag) => {
            if (tag.tagName.text === "defaultValue") {
              // @ts-expect-error TypeScript doesn't want us to be able to assign to a readonly value here
              // eslint-disable-next-line @typescript-eslint/no-unsafe-assignment, @typescript-eslint/no-unsafe-call -- we're messing with TypeScript's internals here
              tag.tagName.escapedText = tag.tagName.escapedText.replace(
                "defaultValue",
                "default"
              );
              if (typeof tag.comment === "string") {
                // @ts-expect-error TypeScript doesn't want us to be able to assign to a readonly value here
                tag.comment = tag.comment.replaceAll("`", "");
              }
            }
          });
        }
      }
    });
  }

  return ts.visitEachChild(
    node,
    (child) => replaceJSDoc(child, context),
    context
  );
};

const updateJSDoc = (program: ts.Program) => {
  const sourceFiles = program.getSourceFiles();
  sourceFiles
    .filter((sourceFile) => sourceFile.fileName.includes(packageRoot))
    .forEach((sourceFile) => {
      ts.transform(sourceFile, [
        (context) => (rootNode) => {
          rootNode.forEachChild((node) => {
            replaceJSDoc(node, context);
          });
          return rootNode;
        },
      ]);
    });
};

const create = (fileName: string, typeName: string) => {
  const config: CompletedConfig = {
    ...DEFAULT_CONFIG,
    path: join(packageRoot, "index.ts"),
    tsconfig: join(__dirname, "../tsconfig.json"),
    type: "Schema",
  };

  const program = createProgram(config);

  updateJSDoc(program);

  const parser = createParser(program, config);

  const formatter = createFormatter(config);
  const generator = new SchemaGenerator(program, parser, formatter, config);
  const schema = generator.createSchema(typeName);
  const filePath = join(__dirname, "..", "schemas", fileName);
  const fileContents = JSON.stringify(schema, null, 2);
  writeFileSync(filePath, fileContents);
};

create("schema.v1.json", "SchemaV1");
create("schema.v2.json", "Schema");
create("schema.json", "Schema");
