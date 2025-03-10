import { EXAMPLES } from "@/example-data/examples";

export function ExamplesTable({
  coreMaintained,
}: {
  coreMaintained?: boolean;
}): JSX.Element {
  return (
    <div className="max-w-full overflow-auto">
      <table>
        <thead>
          <tr>
            <th>Name</th>
            <th>Description</th>
          </tr>
        </thead>
        <tbody>
          {EXAMPLES.filter((meta) =>
            coreMaintained
              ? meta.maintainedByCoreTeam
              : !meta.maintainedByCoreTeam
          ).map((example) => (
            <tr key={example.slug}>
              <td>
                <a
                  href={`https://github.com/vercel/turborepo/tree/main/examples/${example.slug}`}
                  rel="noopener noreferrer"
                  target="_blank"
                >
                  {example.slug}
                </a>
              </td>
              <td className="sm:text-wrap">{example.description}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
