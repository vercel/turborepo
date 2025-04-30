import { rm, mkdir, copyFile, writeFile } from "node:fs/promises";
import path from "node:path";
import type {
  SupportedArch,
  HumanArch,
  Platform,
  SupportedOS,
  NpmOs,
} from "./types";

export const archToHuman: Record<SupportedArch, HumanArch> = {
  x64: "64",
  arm64: "arm64",
};

export const nodeOSLookup: Record<SupportedOS, NpmOs> = {
  darwin: "darwin",
  linux: "linux",
  windows: "win32",
};

const templateDir = path.join(__dirname, "..", "template");

async function generateNativePackage({
  platform,
  version,
  outputDir,
}: {
  platform: Platform;
  version: string;
  outputDir: string;
}) {
  const { os, arch } = platform;
  console.log(`Generating native package for ${os}-${arch}...`);

  console.log(`Cleaning output directory: ${outputDir}`);
  await rm(outputDir, { recursive: true, force: true });
  await mkdir(path.join(outputDir, "bin"), { recursive: true });

  const copyFromTemplate = async (part: string, ...parts: Array<string>) => {
    console.log("Copying ", path.join(part, ...parts));
    await copyFile(
      path.join(templateDir, part, ...parts),
      path.join(outputDir, part, ...parts)
    );
  };

  if (os === "windows") {
    await copyFromTemplate("bin", "turbo");
  }

  await copyFromTemplate("README.md");
  await copyFromTemplate("LICENSE");

  console.log("Generating package.json...");
  const packageJson = {
    name: `turbo-${os}-${archToHuman[arch]}`,
    version,
    description: `The ${os}-${arch} binary for turbo, a monorepo build system.`,
    repository: "https://github.com/vercel/turborepo",
    bugs: "https://github.com/vercel/turborepo/issues",
    homepage: "https://turborepo.com",
    license: "MIT",
    os: [nodeOSLookup[os]],
    cpu: [arch],
    preferUnplugged: true,
  };
  await writeFile(
    path.join(outputDir, "package.json"),
    JSON.stringify(packageJson, null, 2)
  );

  console.log(`Native package generated successfully in ${outputDir}`);
}

// Exported asn an object instead of export keyword, so that these functions
// can be mocked in tests.
export default { generateNativePackage, archToHuman };
