import { rm, mkdir, copyFile, writeFile } from "node:fs/promises";
import path from "node:path";
import type {
  SupportedArch,
  HumanArch,
  Platform,
  SupportedOS,
  NpmOs
} from "./types";

export const archToHuman: Record<SupportedArch, HumanArch> = {
  x64: "64",
  arm64: "arm64"
};

export const nodeOSLookup: Record<SupportedOS, NpmOs> = {
  darwin: "darwin",
  linux: "linux",
  windows: "win32"
};

const templateDir = path.join(__dirname, "..", "template");

async function generateNativePackage({
  platform,
  version,
  outputDir,
  outputBaseDir,
  packagePrefix = "turbo",
  description
}: {
  platform: Platform;
  version: string;
  outputDir: string;
  outputBaseDir: string;
  packagePrefix?: string;
  description?: string;
}) {
  const { os, arch } = platform;
  const safeOutputDir = resolveOutputDir(outputDir, outputBaseDir);
  console.log(`Generating native package for ${os}-${arch}...`);

  console.log(`Cleaning output directory: ${safeOutputDir}`);
  await rm(safeOutputDir, { recursive: true, force: true });
  await mkdir(path.join(safeOutputDir, "bin"), { recursive: true });

  const copyFromTemplate = async (part: string, ...parts: Array<string>) => {
    console.log("Copying ", path.join(part, ...parts));
    await copyFile(
      path.join(templateDir, part, ...parts),
      path.join(safeOutputDir, part, ...parts)
    );
  };

  if (os === "windows") {
    await copyFromTemplate("bin", "turbo");
  }

  await copyFromTemplate("README.md");
  await copyFromTemplate("LICENSE");

  console.log("Generating package.json...");
  const isScoped = packagePrefix.startsWith("@");
  const separator = isScoped ? "/" : "-";
  const packageJson: Record<string, unknown> = {
    name: `${packagePrefix}${separator}${os}-${archToHuman[arch]}`,
    version,
    description:
      description ||
      `The ${os}-${arch} binary for turbo, a monorepo build system.`,
    repository: "https://github.com/vercel/turborepo",
    bugs: "https://github.com/vercel/turborepo/issues",
    homepage: "https://turborepo.dev",
    license: "MIT",
    os: [nodeOSLookup[os]],
    cpu: [arch],
    preferUnplugged: true
  };
  if (isScoped) {
    packageJson.publishConfig = { access: "public" };
  }
  await writeFile(
    path.join(safeOutputDir, "package.json"),
    JSON.stringify(packageJson, null, 2)
  );

  console.log(`Native package generated successfully in ${safeOutputDir}`);
}

function resolveOutputDir(outputDir: string, outputBaseDir: string) {
  const resolvedOutputDir = path.resolve(outputDir);
  const resolvedOutputBaseDir = path.resolve(outputBaseDir);
  const relativeOutputDir = path.relative(
    resolvedOutputBaseDir,
    resolvedOutputDir
  );

  if (
    relativeOutputDir === "" ||
    relativeOutputDir.startsWith("..") ||
    path.isAbsolute(relativeOutputDir)
  ) {
    throw new Error(
      `Refusing to clean output directory outside package base: ${outputDir}`
    );
  }

  return resolvedOutputDir;
}

// Exported asn an object instead of export keyword, so that these functions
// can be mocked in tests.
export default { generateNativePackage, archToHuman };
