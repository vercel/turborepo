import { visit } from "jsonc-parser";

export function getPipelineDecorationOffsets(json: string): number[] {
  const offsets: number[] = [];
  let depth = -1;
  let hasDecoratedPipeline = false;

  visit(json, {
    onObjectProperty: (property, offset) => {
      if (property === "pipeline" && depth === 0 && !hasDecoratedPipeline) {
        hasDecoratedPipeline = true;
        for (let i = 1; i < 9; i++) {
          offsets.push(offset + i);
        }
      }
    },
    onObjectBegin: () => {
      depth += 1;
    },
    onObjectEnd: () => {
      if (depth < 0) {
        throw Error("imbalanced visitor");
      }

      depth -= 1;
    }
  });

  return offsets;
}
