import type { ReactElement } from "react";
import data from "../../content/examples-data.json";

type ExampleMeta = {
  slug: string;
  name: string;
  description: string;
  maintainedByCoreTeam: boolean;
};

const examples = data as ExampleMeta[];

export function ExamplesTable({
  coreMaintained
}: {
  coreMaintained?: boolean;
}): ReactElement {
  return (
    <div className="overflow-auto max-w-full">
      <table>
        <thead>
          <tr>
            <th>Name</th>
            <th>Description</th>
          </tr>
        </thead>
        <tbody>
          {examples
            .filter((meta) =>
              coreMaintained
                ? meta.maintainedByCoreTeam
                : !meta.maintainedByCoreTeam
            )
            .map((example) => {
              return (
                <tr key={example.slug}>
                  <td>
                    <a
                      href={`https://github.com/vercel/turborepo/tree/main/examples/${example.slug}`}
                      rel="noopener noreferrer"
                      target="_blank"
                    >
                      {example.name}
                    </a>
                  </td>
                  <td className="sm:text-wrap">{example.description}</td>
                </tr>
              );
            })}
        </tbody>
      </table>
    </div>
  );
}
