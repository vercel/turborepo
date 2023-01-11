// @ts-ignore
import { runLoaders } from "next/dist/compiled/loader-runner";

type Result = {
  result: Buffer | string;
  resourceBuffer: Buffer;
  cacheable: boolean;
  fileDependencies: string[];
  missingDependencies: string[];
  contextDependencies: string[];
};

export default async function run(
  resource: string,
  loaders: string[]
): Promise<Result> {
  return new Promise((resolve, reject) => {
    runLoaders(
      {
        resource,
        loaders,
      },
      (err: Error, result: Result) => {
        if (err) {
          reject(err);
          return;
        }

        resolve(result);
      }
    );
  });
}
