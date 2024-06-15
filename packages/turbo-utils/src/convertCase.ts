export interface CaseOptions {
  to: "camel" | "pascal" | "kebab" | "snake";
}

export function convertCase(str: string, opts: CaseOptions = { to: "camel" }) {
  switch (opts.to) {
    case "camel":
      return str.replace(/(?:[-_][a-z])/g, (group) =>
        group.toUpperCase().replace("-", "").replace("_", "")
      );
    default:
      throw new Error("Not implemented");
  }
}
