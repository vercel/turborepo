import { getExamplesList } from "#/utils/getExamplesList";

export const ExamplesTable = () => {
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
          {getExamplesList().map((example) => (
            <tr key={example.slug}>
              <td>
                <a
                  href={`https://github.com/vercel/turbo/tree/main/examples/${example.slug}`}
                  target="_blank"
                  rel="noopener noreferrer"
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
};
