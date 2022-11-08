import type { Rule } from "eslint";
import { Node, MemberExpression } from "estree";
import { RULES } from "../constants";
import getEnvVarDependencies from "../utils/getEnvVarDependencies";

const meta: Rule.RuleMetaData = {
  type: "problem",
  docs: {
    description:
      "Do not allow the use of `process.env` without including the env key in turbo.json",
    category: "Configuration Issues",
    recommended: true,
    url: `https://github.com/vercel/turbo/tree/main/packages/eslint-plugin-turbo/docs/rules/${RULES.noUndeclaredEnvVars}.md`,
  },
  schema: [
    {
      type: "object",
      default: {},
      additionalProperties: false,
      properties: {
        turboConfig: {
          require: false,
          type: "object",
        },
        allowList: {
          default: [],
          type: "array",
          items: {
            type: "string",
          },
        },
      },
    },
  ],
};

function create(context: Rule.RuleContext): Rule.RuleListener {
  const { options } = context;
  const allowList: Array<string> = options?.[0]?.allowList || [];
  const regexAllowList: Array<RegExp> = [];
  allowList.forEach((allowed) => {
    try {
      regexAllowList.push(new RegExp(allowed));
    } catch (err) {
      // log the error, but just move on without this allowList entry
      console.error(`Unable to convert "${allowed}" to regex`);
    }
  });
  const turboConfig = options?.[0]?.turboConfig;
  const turboVars = getEnvVarDependencies({
    cwd: context.getCwd(),
    turboConfig,
  });

  // if this returns null, something went wrong reading from the turbo config
  // (this is different from finding a config with no env vars present, which would
  // return an empty set) - so there is no point continuing if we have nothing to check against
  if (!turboVars) {
    // return of {} bails early from a rule check
    return {};
  }

  const checkKey = (node: Node, envKey?: string) => {
    if (
      envKey &&
      !turboVars.has(envKey) &&
      !regexAllowList.some((regex) => regex.test(envKey))
    ) {
      context.report({
        node,
        message: "${{ envKey }} is not listed as a dependency in turbo.json",
        data: { envKey },
      });
    }
  };

  const isComputed = (
    node: MemberExpression & Rule.NodeParentExtension
  ): boolean => {
    if ("computed" in node.parent) {
      return node.parent.computed;
    }

    return false;
  };

  return {
    MemberExpression(node) {
      // we only care about complete process env declarations and non-computed keys
      if (
        "name" in node.object &&
        "name" in node.property &&
        !isComputed(node)
      ) {
        const objectName = node.object.name;
        const propertyName = node.property.name;

        // we're doing something with process.env
        if (objectName === "process" && propertyName === "env") {
          // destructuring from process.env
          if ("id" in node.parent && node.parent.id?.type === "ObjectPattern") {
            const values = node.parent.id.properties.values();
            Array.from(values).forEach((item) => {
              if ("key" in item && "name" in item.key) {
                checkKey(node.parent, item.key.name);
              }
            });
          }

          // accessing key on process.env
          else if (
            "property" in node.parent &&
            "name" in node.parent.property
          ) {
            checkKey(node.parent, node.parent.property?.name);
          }
        }
      }
    },
  };
}

const rule = { create, meta };
export default rule;
