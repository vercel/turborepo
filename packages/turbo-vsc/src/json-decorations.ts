import { visit } from "jsonc-parser";

const taskDefinitionKeys = ["tasks", "pipeline"] as const;

type TaskDefinitionKey = (typeof taskDefinitionKeys)[number];

export function getTaskDefinitionKeyDecorationOffsets(json: string): number[] {
  const topLevelTaskDefinitionKeyOffsets = new Map<TaskDefinitionKey, number>();
  let depth = -1;

  visit(json, {
    onObjectProperty: (property, offset) => {
      if (depth === 0 && isTaskDefinitionKey(property)) {
        topLevelTaskDefinitionKeyOffsets.set(property, offset);
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

  for (const key of taskDefinitionKeys) {
    const offset = topLevelTaskDefinitionKeyOffsets.get(key);

    if (offset !== undefined) {
      return Array.from({ length: key.length }, (_, i) => offset + i + 1);
    }
  }

  return [];
}

function isTaskDefinitionKey(property: string): property is TaskDefinitionKey {
  return taskDefinitionKeys.includes(property as TaskDefinitionKey);
}
