import type { Rule } from "eslint";
import { Node } from "estree";
import { RULES } from "../constants";
import getEnvVarDependencies from "../utils/getEnvVarDependencies";

const meta: Rule.RuleMetaData = {
  type: "problem",
  docs: {
    description:
      "Do not allow the use of `process.env` without including the env key in turbo.json",
    category: "Configuration Issues",
    recommended: true,
    url: `https://github.com/vercel/turborepo/tree/main/packages/eslint-plugin-turbo/docs/rules/${RULES.noUncachedEnvVars}`,
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
  allowList.forEach((envVar) => {
    try {
      regexAllowList.push(new RegExp(envVar));
    } catch (err) {
      console.error(`Unable to convert "${envVar}" to regex`);
    }
  });
  const turboConfig = options?.[0]?.turboConfig;
  const turboVars = getEnvVarDependencies({ turboConfig });
  if (!turboVars) {
    // bail early
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

  return {
    MemberExpression(node) {
      // we only care about complete process env declarations
      if ("name" in node.object && "name" in node.property) {
        const objectName = node.object.name;
        const propertyName = node.property.name;

        // we're doing something with process.env
        if (objectName === "process" && propertyName === "env") {
          // destructing from process.env
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
