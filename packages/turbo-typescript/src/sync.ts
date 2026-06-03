import { createRequire } from "node:module";
import { promises as fs, realpathSync } from "node:fs";
import path from "node:path";
import fg from "fast-glob";
import yaml from "js-yaml";
import semver from "semver";
import {
  applyEdits,
  findNodeAtLocation,
  modify,
  parse,
  parseTree,
  printParseErrorCode,
  type ParseError
} from "jsonc-parser";
import type {
  CommandName,
  Diagnostic,
  MigrationState,
  PackageManifest,
  ProjectReferencesOptions,
  ProjectReferencesResult,
  WorkspacePackage
} from "./types";
import { ProjectReferencesError } from "./types";

type PackageManager = "npm" | "pnpm" | "yarn" | "bun";

interface TurboConfigFile {
  path: string;
  relativePath: string;
  text: string;
  data: Record<string, unknown>;
}

interface WorkspaceContext {
  root: string;
  rootRealpath: string;
  packageManager: PackageManager;
  turboConfig: TurboConfigFile;
  packages: Array<WorkspacePackage>;
  packagesByName: Map<string, WorkspacePackage>;
  packagesByPath: Map<string, WorkspacePackage>;
  edges: Map<string, Array<string>>;
}

interface GraphState {
  ignored: Array<string>;
  excluded: Array<string>;
  valid: Array<string>;
  cycles: Array<Array<string>>;
  blockers: Map<string, Array<string>>;
  missingTsconfig: Array<string>;
}

interface PlannedFile {
  path: string;
  relativePath: string;
  before: string | undefined;
  after: string;
}

const FORMAT_OPTIONS = { insertSpaces: true, tabSize: 2, eol: "\n" };

export async function initProjectReferences(
  options: ProjectReferencesOptions = {}
): Promise<ProjectReferencesResult> {
  const context = await loadContext(options.cwd);
  const existing = readMigrationState(context.turboConfig, false);
  if (existing.present && !options.force) {
    throw new ProjectReferencesError(
      "typescriptProjectReferences already exists",
      [
        {
          level: "error",
          code: "already_configured",
          message:
            "typescriptProjectReferences already exists. Re-run with --force to recompute it.",
          path: context.turboConfig.relativePath
        }
      ]
    );
  }

  const rootTsconfig = await readJsoncFileIfExists(
    path.join(context.root, "tsconfig.json"),
    "tsconfig.json"
  );
  if (!rootTsconfig) {
    throw new ProjectReferencesError("Missing root tsconfig.json", [
      {
        level: "error",
        code: "missing_root_tsconfig",
        message: "init requires an existing root tsconfig.json.",
        path: "tsconfig.json"
      }
    ]);
  }

  const referenced = resolveReferenceSet(
    context,
    context.root,
    getReferences(rootTsconfig.data)
  );
  const preservedIgnored =
    options.force && existing.present ? existing.state.ignored : [];
  const ignored = sortPaths(
    dedupe([
      ...preservedIgnored.filter((entry) => context.packagesByPath.has(entry)),
      ...context.packages
        .filter((pkg) => !pkg.hasTsconfig)
        .map((pkg) => pkg.relativePath)
    ])
  );
  const excluded = sortPaths(
    context.packages
      .filter(
        (pkg) =>
          !ignored.includes(pkg.relativePath) &&
          !referenced.has(pkg.relativePath)
      )
      .map((pkg) => pkg.relativePath)
  );
  const state = { ignored, excluded };
  const graph = computeGraphState(context, state);
  const plans: Array<PlannedFile> = [
    planTurboConfig(context.turboConfig, normalizeMigrationState(state))
  ];

  for (const pkgPath of sortPaths([...referenced])) {
    const pkg = context.packagesByPath.get(pkgPath);
    if (!pkg || ignored.includes(pkgPath) || !pkg.hasTsconfig) {
      continue;
    }
    plans.push(await planPackageTsconfig(context, pkg, graph.valid));
  }

  return applyPlans({
    command: "init",
    context,
    graph,
    plans,
    dryRun: options.dryRun === true,
    diagnostics: diagnosticsForInit(state)
  });
}

export async function checkProjectReferences(
  options: ProjectReferencesOptions = {}
): Promise<ProjectReferencesResult> {
  const context = await loadContext(options.cwd);
  const parsed = readMigrationState(context.turboConfig, true);
  validateKnownConfigPaths(context, parsed.state);
  const graph = computeGraphState(context, parsed.state);
  const plans = await planConvergedFiles(context, graph, false);
  const changedPlans = plans.filter((plan) => plan.before !== plan.after);
  const diagnostics = diagnosticsForState(graph, parsed.state, "would");

  for (const plan of changedPlans) {
    diagnostics.push({
      level: "error",
      code: "config_out_of_sync",
      message: `${plan.relativePath} differs from the desired Project References state.`,
      path: plan.relativePath
    });
  }

  if (changedPlans.length > 0) {
    diagnostics.push({
      level: "info",
      code: "next_action",
      message: "Run turbo-typescript project-references write."
    });
  }

  const result = createResult({
    command: "check",
    context,
    graph,
    dryRun: true,
    changedFiles: changedPlans.map((plan) => plan.relativePath),
    diagnostics,
    success: changedPlans.length === 0
  });
  if (changedPlans.length > 0) {
    result.success = false;
  }
  return result;
}

export async function writeProjectReferences(
  options: ProjectReferencesOptions = {}
): Promise<ProjectReferencesResult> {
  const context = await loadContext(options.cwd);
  const parsed = readMigrationState(context.turboConfig, true);
  validateKnownConfigPaths(context, parsed.state);
  const graph = computeGraphState(context, parsed.state);
  const plans = await planConvergedFiles(context, graph, true);

  return applyPlans({
    command: "write",
    context,
    graph,
    plans,
    dryRun: options.dryRun === true,
    diagnostics: diagnosticsForState(
      graph,
      parsed.state,
      options.dryRun === true ? "would" : "did"
    )
  });
}

export async function getProjectReferenceCandidates(
  options: ProjectReferencesOptions = {}
): Promise<ProjectReferencesResult> {
  const context = await loadContext(options.cwd);
  const parsed = readMigrationState(context.turboConfig, true);
  const graph = computeGraphState(context, parsed.state);
  const currentExcluded = new Set(parsed.state.excluded);
  const candidates = graph.valid.filter((pkgPath) =>
    currentExcluded.has(pkgPath)
  );
  const newPackages = graph.excluded.filter(
    (pkgPath) =>
      !currentExcluded.has(pkgPath) && !parsed.state.ignored.includes(pkgPath)
  );
  const diagnostics = diagnosticsForState(graph, parsed.state, "can");

  if (candidates.length === 0) {
    diagnostics.push({
      level: "info",
      code: "no_candidates",
      message: "No excluded packages can be migrated right now."
    });
  } else {
    diagnostics.push({
      level: "info",
      code: "next_action",
      message:
        "Run turbo-typescript project-references write to migrate candidates.",
      details: candidates
    });
  }

  return createResult({
    command: "candidates",
    context,
    graph,
    dryRun: true,
    changedFiles: [],
    diagnostics,
    success: true,
    candidates,
    newPackages
  });
}

async function loadContext(cwd?: string): Promise<WorkspaceContext> {
  const root = path.resolve(cwd ?? process.cwd());
  const rootRealpath = await realpath(root);
  const turboConfig = await readTurboConfig(root);
  const rootManifest = await readPackageManifest(root, "package.json");
  const packageManager = await detectPackageManager(root, rootManifest);
  const workspacePatterns = await readWorkspacePatterns(root, rootManifest);
  const packages = await discoverPackages(
    root,
    rootRealpath,
    workspacePatterns
  );

  if (packages.length === 0) {
    throw new ProjectReferencesError("No workspace packages found", [
      {
        level: "error",
        code: "single_package_workspace",
        message: "Non-multi-package workspaces are not supported."
      }
    ]);
  }

  const packagesByName = new Map<string, WorkspacePackage>();
  const packagesByPath = new Map<string, WorkspacePackage>();
  for (const pkg of packages) {
    if (packagesByName.has(pkg.name)) {
      throw new ProjectReferencesError(
        `Duplicate workspace package name ${pkg.name}`,
        [
          {
            level: "error",
            code: "duplicate_package_name",
            message: `Duplicate workspace package name ${pkg.name}.`,
            packagePath: pkg.relativePath
          }
        ]
      );
    }
    packagesByName.set(pkg.name, pkg);
    packagesByPath.set(pkg.relativePath, pkg);
  }

  const edges = buildEdges(packageManager, packages, packagesByName);

  return {
    root,
    rootRealpath,
    packageManager,
    turboConfig,
    packages,
    packagesByName,
    packagesByPath,
    edges
  };
}

async function readTurboConfig(root: string): Promise<TurboConfigFile> {
  const jsonPath = path.join(root, "turbo.json");
  const jsoncPath = path.join(root, "turbo.jsonc");
  const [jsonExists, jsoncExists] = await Promise.all([
    pathExists(jsonPath),
    pathExists(jsoncPath)
  ]);

  if (jsonExists && jsoncExists) {
    throw new ProjectReferencesError("Both turbo.json and turbo.jsonc exist", [
      {
        level: "error",
        code: "conflicting_turbo_configs",
        message:
          "Only one root Turbo config is supported. Remove turbo.json or turbo.jsonc."
      }
    ]);
  }

  if (!jsonExists && !jsoncExists) {
    throw new ProjectReferencesError("Missing root Turbo config", [
      {
        level: "error",
        code: "missing_turbo_config",
        message: "Expected a root turbo.json or turbo.jsonc."
      }
    ]);
  }

  const filePath = jsonExists ? jsonPath : jsoncPath;
  const relativePath = jsonExists ? "turbo.json" : "turbo.jsonc";
  const file = await readJsoncFile(filePath, relativePath);
  return { ...file, relativePath };
}

async function readWorkspacePatterns(
  root: string,
  rootManifest: PackageManifest
): Promise<Array<string>> {
  const pnpmWorkspacePath = path.join(root, "pnpm-workspace.yaml");
  if (await pathExists(pnpmWorkspacePath)) {
    const text = await fs.readFile(pnpmWorkspacePath, "utf8");
    const data = yaml.load(text) as { packages?: unknown } | undefined;
    if (!data || !Array.isArray(data.packages)) {
      throw new ProjectReferencesError("Invalid pnpm-workspace.yaml", [
        {
          level: "error",
          code: "invalid_workspace_config",
          message: "pnpm-workspace.yaml must contain a packages array.",
          path: "pnpm-workspace.yaml"
        }
      ]);
    }
    return data.packages.map(assertWorkspacePattern);
  }

  const workspaces = rootManifest.workspaces;
  if (Array.isArray(workspaces)) {
    return workspaces.map(assertWorkspacePattern);
  }
  if (workspaces && Array.isArray(workspaces.packages)) {
    return workspaces.packages.map(assertWorkspacePattern);
  }

  throw new ProjectReferencesError("Missing workspace configuration", [
    {
      level: "error",
      code: "missing_workspace_config",
      message:
        "Expected pnpm-workspace.yaml packages or package.json workspaces."
    }
  ]);
}

function assertWorkspacePattern(value: unknown): string {
  if (typeof value !== "string") {
    throw new ProjectReferencesError("Workspace patterns must be strings");
  }
  const pattern = value.startsWith("!") ? value.slice(1) : value;
  const normalized = toPosix(pattern.replace(/^\.\//, ""));
  if (
    path.isAbsolute(pattern) ||
    normalized === ".." ||
    normalized.startsWith("../") ||
    normalized.includes("/../")
  ) {
    throw new ProjectReferencesError(`Unsafe workspace glob ${value}`, [
      {
        level: "error",
        code: "unsafe_workspace_glob",
        message: `Workspace glob ${value} escapes the workspace root.`
      }
    ]);
  }
  return value;
}

async function discoverPackages(
  root: string,
  rootRealpath: string,
  patterns: Array<string>
): Promise<Array<WorkspacePackage>> {
  const positivePatterns = patterns
    .filter((pattern) => !pattern.startsWith("!"))
    .map((pattern) => `${pattern.replace(/\/$/, "")}/package.json`);
  const ignorePatterns = patterns
    .filter((pattern) => pattern.startsWith("!"))
    .map((pattern) => `${pattern.slice(1).replace(/\/$/, "")}/**`);
  const manifestPaths = await fg(positivePatterns, {
    cwd: root,
    ignore: ignorePatterns,
    onlyFiles: true,
    dot: true,
    unique: true,
    followSymbolicLinks: false
  });

  const packages = await Promise.all(
    manifestPaths
      .filter((manifestPath) => toPosix(path.dirname(manifestPath)) !== ".")
      .sort()
      .map(async (manifestPath) => {
        const relativePath = toPosix(path.dirname(manifestPath));
        const dir = path.join(root, relativePath);
        const realDir = await realpath(dir);
        assertInsideRoot(rootRealpath, realDir, relativePath);
        const manifest = await readPackageManifest(
          dir,
          `${relativePath}/package.json`
        );
        if (!manifest.name) {
          throw new ProjectReferencesError(
            "Workspace package is missing name",
            [
              {
                level: "error",
                code: "nameless_package",
                message: "Workspace packages must have a package.json name.",
                path: `${relativePath}/package.json`,
                packagePath: relativePath
              }
            ]
          );
        }
        return {
          name: manifest.name,
          version: manifest.version ?? "0.0.0",
          dir,
          relativePath,
          manifest,
          hasTsconfig: await pathExists(path.join(dir, "tsconfig.json"))
        } satisfies WorkspacePackage;
      })
  );

  return packages.sort((a, b) => a.relativePath.localeCompare(b.relativePath));
}

async function readPackageManifest(
  dir: string,
  relativePath: string
): Promise<PackageManifest> {
  const filePath = path.join(dir, "package.json");
  try {
    const text = await fs.readFile(filePath, "utf8");
    return JSON.parse(text) as PackageManifest;
  } catch (error) {
    throw new ProjectReferencesError(`Failed to read ${relativePath}`, [
      {
        level: "error",
        code: "malformed_package_json",
        message:
          error instanceof Error
            ? error.message
            : `Failed to read ${relativePath}.`,
        path: relativePath
      }
    ]);
  }
}

async function detectPackageManager(
  root: string,
  rootManifest: PackageManifest
): Promise<PackageManager> {
  const packageManager = rootManifest.packageManager?.split("@")[0];
  if (
    packageManager === "npm" ||
    packageManager === "pnpm" ||
    packageManager === "yarn" ||
    packageManager === "bun"
  ) {
    return packageManager;
  }

  const signals: Array<[PackageManager, string]> = [
    ["pnpm", "pnpm-lock.yaml"],
    ["bun", "bun.lockb"],
    ["bun", "bun.lock"],
    ["yarn", "yarn.lock"],
    ["npm", "package-lock.json"],
    ["npm", "npm-shrinkwrap.json"]
  ];
  for (const [manager, file] of signals) {
    if (await pathExists(path.join(root, file))) {
      return manager;
    }
  }
  return "npm";
}

function buildEdges(
  packageManager: PackageManager,
  packages: Array<WorkspacePackage>,
  packagesByName: Map<string, WorkspacePackage>
): Map<string, Array<string>> {
  const edges = new Map<string, Array<string>>();
  for (const pkg of packages) {
    const deps = {
      ...pkg.manifest.dependencies,
      ...pkg.manifest.devDependencies
    };
    const directEdges = new Set<string>();
    for (const [name, specifier] of Object.entries(deps)) {
      const dependency = packagesByName.get(name);
      if (!dependency) {
        continue;
      }
      if (isWorkspaceLink(packageManager, specifier, dependency.version)) {
        directEdges.add(dependency.relativePath);
      }
    }
    edges.set(pkg.relativePath, sortPaths([...directEdges]));
  }
  return edges;
}

function isWorkspaceLink(
  packageManager: PackageManager,
  specifier: string,
  dependencyVersion: string
): boolean {
  if (specifier.startsWith("workspace:")) {
    return true;
  }
  if (specifier.startsWith("link:") || specifier.startsWith("file:")) {
    return true;
  }
  if (packageManager === "pnpm" && specifier === "*") {
    return false;
  }
  const minimum = semver.minVersion(specifier);
  if (!minimum) {
    return false;
  }
  return semver.satisfies(dependencyVersion, specifier, {
    includePrerelease: true
  });
}

function validateKnownConfigPaths(
  context: WorkspaceContext,
  state: MigrationState
) {
  const unknown = [...state.excluded, ...state.ignored].filter(
    (entry) => !context.packagesByPath.has(entry)
  );
  if (unknown.length === 0) {
    return;
  }
  throw new ProjectReferencesError("Unknown package paths in config", [
    {
      level: "error",
      code: "unknown_config_paths",
      message:
        "typescriptProjectReferences contains paths that are not workspace packages.",
      path: context.turboConfig.relativePath,
      details: sortPaths(unknown)
    }
  ]);
}

function computeGraphState(
  context: WorkspaceContext,
  inputState: MigrationState
): GraphState {
  const packagePaths = new Set(context.packages.map((pkg) => pkg.relativePath));
  const ignored = sortPaths(
    inputState.ignored.filter((entry) => packagePaths.has(entry))
  );
  const missingTsconfig = context.packages
    .filter((pkg) => !ignored.includes(pkg.relativePath) && !pkg.hasTsconfig)
    .map((pkg) => pkg.relativePath);
  const effectiveIgnored = sortPaths(dedupe([...ignored, ...missingTsconfig]));
  const ignoredSet = new Set(effectiveIgnored);
  const cycles = findCycles(context, ignoredSet);
  const cycleNodes = new Set(cycles.flat());
  const valid = new Set(
    context.packages
      .filter(
        (pkg) =>
          !ignoredSet.has(pkg.relativePath) &&
          pkg.hasTsconfig &&
          !cycleNodes.has(pkg.relativePath)
      )
      .map((pkg) => pkg.relativePath)
  );

  let changed = true;
  while (changed) {
    changed = false;
    for (const pkgPath of Array.from(valid)) {
      const deps = context.edges.get(pkgPath) ?? [];
      if (deps.some((dep) => !ignoredSet.has(dep) && !valid.has(dep))) {
        valid.delete(pkgPath);
        changed = true;
      }
    }
  }

  const validList = sortPaths([...valid]);
  const excluded = sortPaths(
    context.packages
      .filter(
        (pkg) =>
          !ignoredSet.has(pkg.relativePath) && !valid.has(pkg.relativePath)
      )
      .map((pkg) => pkg.relativePath)
  );
  const blockers = new Map<string, Array<string>>();
  for (const pkgPath of excluded) {
    const deps = context.edges.get(pkgPath) ?? [];
    const blockedBy = deps.filter(
      (dep) => !ignoredSet.has(dep) && !valid.has(dep)
    );
    if (blockedBy.length > 0) {
      blockers.set(pkgPath, blockedBy);
    }
  }

  return {
    ignored: effectiveIgnored,
    excluded,
    valid: validList,
    cycles,
    blockers,
    missingTsconfig
  };
}

function findCycles(
  context: WorkspaceContext,
  ignored: Set<string>
): Array<Array<string>> {
  const indexByPath = new Map<string, number>();
  const lowlink = new Map<string, number>();
  const stack: Array<string> = [];
  const onStack = new Set<string>();
  const cycles: Array<Array<string>> = [];
  let index = 0;

  const strongConnect = (node: string) => {
    indexByPath.set(node, index);
    lowlink.set(node, index);
    index += 1;
    stack.push(node);
    onStack.add(node);

    for (const dep of context.edges.get(node) ?? []) {
      if (ignored.has(dep)) {
        continue;
      }
      if (!indexByPath.has(dep)) {
        strongConnect(dep);
        lowlink.set(
          node,
          Math.min(lowlink.get(node) ?? 0, lowlink.get(dep) ?? 0)
        );
      } else if (onStack.has(dep)) {
        lowlink.set(
          node,
          Math.min(lowlink.get(node) ?? 0, indexByPath.get(dep) ?? 0)
        );
      }
    }

    if (lowlink.get(node) === indexByPath.get(node)) {
      const component: Array<string> = [];
      let current: string | undefined;
      do {
        current = stack.pop();
        if (current) {
          onStack.delete(current);
          component.push(current);
        }
      } while (current && current !== node);

      const selfCycle = (context.edges.get(node) ?? []).includes(node);
      if (component.length > 1 || selfCycle) {
        cycles.push(sortPaths(component));
      }
    }
  };

  for (const pkg of context.packages) {
    if (!ignored.has(pkg.relativePath) && !indexByPath.has(pkg.relativePath)) {
      strongConnect(pkg.relativePath);
    }
  }

  return cycles.sort((a, b) => a[0].localeCompare(b[0]));
}

async function planConvergedFiles(
  context: WorkspaceContext,
  graph: GraphState,
  createRootTsconfig: boolean
): Promise<Array<PlannedFile>> {
  const state = normalizeMigrationState({
    ignored: graph.ignored,
    excluded: graph.excluded
  });
  const plans: Array<PlannedFile> = [
    planTurboConfig(context.turboConfig, state)
  ];
  plans.push(await planRootTsconfig(context, graph.valid, createRootTsconfig));
  for (const pkgPath of graph.valid) {
    const pkg = context.packagesByPath.get(pkgPath);
    if (pkg) {
      plans.push(await planPackageTsconfig(context, pkg, graph.valid));
    }
  }
  return plans;
}

function planTurboConfig(
  config: TurboConfigFile,
  state: MigrationState | true
): PlannedFile {
  const after = setJsonValue(
    config.text,
    ["typescriptProjectReferences"],
    state
  );
  return {
    path: config.path,
    relativePath: config.relativePath,
    before: config.text,
    after
  };
}

async function planRootTsconfig(
  context: WorkspaceContext,
  valid: Array<string>,
  createIfMissing: boolean
): Promise<PlannedFile> {
  const filePath = path.join(context.root, "tsconfig.json");
  const current = await readJsoncFileIfExists(filePath, "tsconfig.json");
  if (!current && !createIfMissing) {
    throw new ProjectReferencesError("Missing root tsconfig.json", [
      {
        level: "error",
        code: "missing_root_tsconfig",
        message: "check requires an existing root tsconfig.json.",
        path: "tsconfig.json"
      }
    ]);
  }

  let text = current?.text ?? '{\n  "files": [],\n  "references": []\n}\n';
  text = setJsonValue(text, ["files"], []);
  text = removeJsonValue(text, ["include"]);
  text = setJsonValue(
    text,
    ["references"],
    buildReferenceObjects(current?.data, context, context.root, valid, true)
  );
  return {
    path: filePath,
    relativePath: "tsconfig.json",
    before: current?.text,
    after: text
  };
}

async function planPackageTsconfig(
  context: WorkspaceContext,
  pkg: WorkspacePackage,
  valid: Array<string>
): Promise<PlannedFile> {
  const filePath = path.join(pkg.dir, "tsconfig.json");
  const relativePath = `${pkg.relativePath}/tsconfig.json`;
  const current = await readJsoncFile(filePath, relativePath);
  const deps = (context.edges.get(pkg.relativePath) ?? []).filter((dep) =>
    valid.includes(dep)
  );
  let text = current.text;
  text = setJsonValue(
    text,
    ["references"],
    buildReferenceObjects(current.data, context, pkg.dir, deps, false)
  );
  if (!(await effectiveComposite(context.root, filePath))) {
    text = setJsonValue(text, ["compilerOptions", "composite"], true);
  }
  return {
    path: filePath,
    relativePath,
    before: current.text,
    after: text
  };
}

function buildReferenceObjects(
  currentConfig: Record<string, unknown> | undefined,
  context: WorkspaceContext,
  fromDir: string,
  desiredPackagePaths: Array<string>,
  rootReferences: boolean
): Array<Record<string, unknown>> {
  const existing = new Map<string, Record<string, unknown>>();
  const references = getReferences(currentConfig ?? {});
  for (const ref of references) {
    const refPath = typeof ref.path === "string" ? ref.path : undefined;
    if (!refPath) {
      continue;
    }
    const resolved = resolvePackageReference(context, fromDir, refPath);
    if (resolved) {
      existing.set(resolved.relativePath, ref);
    }
  }

  return sortPaths(desiredPackagePaths).map((pkgPath) => {
    const pkg = context.packagesByPath.get(pkgPath);
    const referencePath = rootReferences
      ? pkgPath
      : toPosix(
          path.relative(fromDir, pkg?.dir ?? path.join(context.root, pkgPath))
        ) || ".";
    const preserved = existing.get(pkgPath) ?? {};
    return { ...preserved, path: referencePath };
  });
}

function readMigrationState(
  config: TurboConfigFile,
  required: boolean
): { present: boolean; state: MigrationState } {
  const raw = config.data.typescriptProjectReferences;
  if (raw === undefined) {
    if (required) {
      throw new ProjectReferencesError(
        "typescriptProjectReferences is absent",
        [
          {
            level: "error",
            code: "not_configured",
            message:
              "typescriptProjectReferences is absent. Run turbo-typescript project-references init first.",
            path: config.relativePath
          }
        ]
      );
    }
    return { present: false, state: { excluded: [], ignored: [] } };
  }
  if (raw === true || (isRecord(raw) && Object.keys(raw).length === 0)) {
    return { present: true, state: { excluded: [], ignored: [] } };
  }
  if (!isRecord(raw)) {
    throw new ProjectReferencesError(
      "Invalid typescriptProjectReferences config",
      [
        {
          level: "error",
          code: "invalid_config",
          message: "typescriptProjectReferences must be true or an object.",
          path: config.relativePath
        }
      ]
    );
  }
  const excluded = readPathArray(raw.excluded, "excluded", config.relativePath);
  const ignored = readPathArray(raw.ignored, "ignored", config.relativePath);
  return { present: true, state: { excluded, ignored } };
}

function readPathArray(
  raw: unknown,
  field: string,
  configPath: string
): Array<string> {
  if (raw === undefined) {
    return [];
  }
  if (!Array.isArray(raw) || raw.some((entry) => typeof entry !== "string")) {
    throw new ProjectReferencesError(`Invalid ${field} config`, [
      {
        level: "error",
        code: "invalid_config",
        message: `${field} must be an array of workspace-relative package paths.`,
        path: configPath
      }
    ]);
  }
  return sortPaths(dedupe(raw.map(normalizeConfigPath)));
}

function normalizeConfigPath(input: string): string {
  if (path.isAbsolute(input)) {
    throw new ProjectReferencesError(
      `Absolute config path ${input} is not allowed`
    );
  }
  const normalized = toPosix(
    path.posix.normalize(toPosix(input).replace(/^\.\//, ""))
  ).replace(/\/$/, "");
  if (
    normalized === "." ||
    normalized === ".." ||
    normalized.startsWith("../") ||
    normalized.includes("/../")
  ) {
    throw new ProjectReferencesError(`Unsafe config path ${input}`);
  }
  return normalized;
}

function normalizeMigrationState(state: MigrationState): MigrationState | true {
  const excluded = sortPaths(dedupe(state.excluded));
  const ignored = sortPaths(dedupe(state.ignored));
  if (excluded.length === 0 && ignored.length === 0) {
    return true;
  }
  const normalized: Partial<MigrationState> = {};
  if (excluded.length > 0) {
    normalized.excluded = excluded;
  }
  if (ignored.length > 0) {
    normalized.ignored = ignored;
  }
  return normalized as MigrationState;
}

function diagnosticsForState(
  graph: GraphState,
  previous: MigrationState,
  mode: "can" | "would" | "did"
): Array<Diagnostic> {
  const diagnostics: Array<Diagnostic> = [];
  const previousExcluded = new Set(previous.excluded);
  const previousIgnored = new Set(previous.ignored);
  const removedExcluded = previous.excluded.filter(
    (pkgPath) => !graph.excluded.includes(pkgPath)
  );
  const addedExcluded = graph.excluded.filter(
    (pkgPath) => !previousExcluded.has(pkgPath)
  );
  const addedIgnored = graph.ignored.filter(
    (pkgPath) => !previousIgnored.has(pkgPath)
  );

  if (removedExcluded.length > 0) {
    diagnostics.push({
      level: "info",
      code: "removed_from_excluded",
      message: removedFromExcludedMessage(mode),
      details: removedExcluded
    });
  }
  if (addedExcluded.length > 0) {
    diagnostics.push({
      level: "warning",
      code: "added_to_excluded",
      message: "Packages must remain excluded to preserve a valid graph.",
      details: addedExcluded
    });
  }
  if (addedIgnored.length > 0) {
    diagnostics.push({
      level: "info",
      code: "added_to_ignored",
      message: "Packages without package-root tsconfig.json are ignored.",
      details: addedIgnored
    });
  }
  for (const cycle of graph.cycles) {
    diagnostics.push({
      level: "warning",
      code: "cycle",
      message:
        "TypeScript Project References do not support dependency cycles.",
      details: cycle
    });
  }
  for (const [pkgPath, blockers] of graph.blockers) {
    diagnostics.push({
      level: "warning",
      code: "blocked_by_dependencies",
      message: `${pkgPath} is blocked by excluded dependencies.`,
      packagePath: pkgPath,
      details: blockers
    });
  }
  return diagnostics;
}

function removedFromExcludedMessage(mode: "can" | "would" | "did"): string {
  switch (mode) {
    case "can": {
      return "Packages can be removed from excluded.";
    }
    case "would": {
      return "Packages would be removed from excluded.";
    }
    case "did": {
      return "Packages removed from excluded.";
    }
  }
}

function diagnosticsForInit(state: MigrationState): Array<Diagnostic> {
  const diagnostics: Array<Diagnostic> = [];
  if (state.excluded.length > 0) {
    diagnostics.push({
      level: "info",
      code: "added_to_excluded",
      message: "Packages will start in excluded until write can migrate them.",
      details: state.excluded
    });
  }
  if (state.ignored.length > 0) {
    diagnostics.push({
      level: "info",
      code: "added_to_ignored",
      message: "Packages without package-root tsconfig.json are ignored.",
      details: state.ignored
    });
  }
  return diagnostics;
}

async function applyPlans({
  command,
  context,
  graph,
  plans,
  dryRun,
  diagnostics
}: {
  command: CommandName;
  context: WorkspaceContext;
  graph: GraphState;
  plans: Array<PlannedFile>;
  dryRun: boolean;
  diagnostics: Array<Diagnostic>;
}): Promise<ProjectReferencesResult> {
  const changed = plans.filter((plan) => plan.before !== plan.after);
  const written: Array<string> = [];
  if (!dryRun) {
    for (const plan of changed) {
      try {
        await fs.writeFile(plan.path, plan.after);
        written.push(plan.relativePath);
      } catch (error) {
        throw new ProjectReferencesError("Failed while writing files", [
          {
            level: "error",
            code: "write_failed",
            message:
              error instanceof Error
                ? error.message
                : "Failed while writing files.",
            path: plan.relativePath,
            details: written
          }
        ]);
      }
    }
  }

  return createResult({
    command,
    context,
    graph,
    dryRun,
    changedFiles: changed.map((plan) => plan.relativePath),
    diagnostics,
    success: true
  });
}

function createResult({
  command,
  context,
  graph,
  dryRun,
  changedFiles,
  diagnostics,
  success,
  candidates,
  newPackages
}: {
  command: CommandName;
  context: WorkspaceContext;
  graph: GraphState;
  dryRun: boolean;
  changedFiles: Array<string>;
  diagnostics: Array<Diagnostic>;
  success: boolean;
  candidates?: Array<string>;
  newPackages?: Array<string>;
}): ProjectReferencesResult {
  const candidateList =
    candidates ??
    graph.valid.filter((pkgPath) => graph.excluded.includes(pkgPath));
  return {
    version: 1,
    command,
    success,
    dryRun,
    changedFiles: sortPaths(changedFiles),
    diagnostics,
    summary: {
      packageCount: context.packages.length,
      validCount: graph.valid.length,
      excludedCount: graph.excluded.length,
      ignoredCount: graph.ignored.length,
      candidateCount: candidateList.length
    },
    candidates: sortPaths(candidateList),
    newPackages: sortPaths(newPackages ?? [])
  };
}

async function effectiveComposite(
  root: string,
  configPath: string
): Promise<boolean> {
  const ts = resolveTypeScript(root);
  const rootRealpath = realpathSync(root);
  const parsed = ts.getParsedCommandLineOfConfigFile(
    configPath,
    {},
    {
      ...ts.sys,
      fileExists: (fileName) => {
        assertTypeScriptConfigInsideRoot(rootRealpath, fileName);
        return ts.sys.fileExists(fileName);
      },
      readFile: (fileName) => {
        assertTypeScriptConfigInsideRoot(rootRealpath, fileName);
        return ts.sys.readFile(fileName);
      },
      onUnRecoverableConfigFileDiagnostic: () => undefined,
      readDirectory: () => []
    }
  );
  return parsed?.options?.composite === true;
}

function assertTypeScriptConfigInsideRoot(
  rootRealpath: string,
  fileName: string
) {
  if (!fileName.endsWith(".json") || !pathExistsSync(fileName)) {
    return;
  }
  const fileRealpath = realpathSync(fileName);
  const relative = path.relative(rootRealpath, fileRealpath);
  if (
    relative === "" ||
    (!relative.startsWith("..") && !path.isAbsolute(relative))
  ) {
    return;
  }
  throw new ProjectReferencesError(
    "TypeScript config extends outside workspace",
    [
      {
        level: "error",
        code: "external_extends",
        message: "TypeScript resolved an extends file outside the workspace."
      }
    ]
  );
}

function resolveTypeScript(root: string): typeof import("typescript") {
  try {
    const requireFromRoot = createRequire(path.join(root, "package.json"));
    return requireFromRoot("typescript") as typeof import("typescript");
  } catch {
    throw new ProjectReferencesError("Unable to resolve TypeScript", [
      {
        level: "error",
        code: "missing_typescript",
        message:
          "Install typescript in the workspace root. The CLI resolves TypeScript from the target workspace."
      }
    ]);
  }
}

function getReferences(
  config: Record<string, unknown>
): Array<Record<string, unknown>> {
  const references = config.references;
  if (!Array.isArray(references)) {
    return [];
  }
  return references.filter(isRecord);
}

function resolveReferenceSet(
  context: WorkspaceContext,
  fromDir: string,
  references: Array<Record<string, unknown>>
): Set<string> {
  const resolved = new Set<string>();
  for (const reference of references) {
    if (typeof reference.path !== "string") {
      continue;
    }
    const pkg = resolvePackageReference(context, fromDir, reference.path);
    if (pkg) {
      resolved.add(pkg.relativePath);
    }
  }
  return resolved;
}

function resolvePackageReference(
  context: WorkspaceContext,
  fromDir: string,
  referencePath: string
): WorkspacePackage | undefined {
  const absolute = path.resolve(fromDir, referencePath);
  const normalized = toPosix(path.relative(context.root, absolute));
  return context.packagesByPath.get(normalized);
}

async function readJsoncFile(filePath: string, relativePath: string) {
  const text = await fs.readFile(filePath, "utf8");
  const errors: Array<ParseError> = [];
  const data = parse(text, errors, { allowTrailingComma: true }) as Record<
    string,
    unknown
  >;
  if (errors.length > 0) {
    throw parseJsonError(relativePath, errors[0]);
  }
  if (!isRecord(data)) {
    throw new ProjectReferencesError(
      `${relativePath} must contain a JSON object`,
      [
        {
          level: "error",
          code: "invalid_json",
          message: `${relativePath} must contain a JSON object.`,
          path: relativePath
        }
      ]
    );
  }
  return { path: filePath, relativePath, text, data };
}

async function readJsoncFileIfExists(filePath: string, relativePath: string) {
  if (!(await pathExists(filePath))) {
    return undefined;
  }
  return readJsoncFile(filePath, relativePath);
}

function parseJsonError(
  relativePath: string,
  error: ParseError
): ProjectReferencesError {
  return new ProjectReferencesError(`Malformed JSONC in ${relativePath}`, [
    {
      level: "error",
      code: "malformed_jsonc",
      message: `${relativePath}: ${printParseErrorCode(error.error)} at offset ${error.offset}.`,
      path: relativePath
    }
  ]);
}

function setJsonValue(
  text: string,
  jsonPath: Array<string>,
  value: unknown
): string {
  const edits = modify(text, jsonPath, value, {
    formattingOptions: FORMAT_OPTIONS
  });
  return applyEdits(text, edits);
}

function removeJsonValue(text: string, jsonPath: Array<string>): string {
  const tree = parseTree(text);
  if (!tree || !findNodeAtLocation(tree, jsonPath)) {
    return text;
  }
  const edits = modify(text, jsonPath, undefined, {
    formattingOptions: FORMAT_OPTIONS
  });
  return applyEdits(text, edits);
}

async function pathExists(filePath: string): Promise<boolean> {
  try {
    await fs.access(filePath);
    return true;
  } catch {
    return false;
  }
}

function pathExistsSync(filePath: string): boolean {
  try {
    realpathSync(filePath);
    return true;
  } catch {
    return false;
  }
}

async function realpath(filePath: string): Promise<string> {
  return fs.realpath(filePath);
}

function assertInsideRoot(
  rootRealpath: string,
  targetRealpath: string,
  relativePath: string
) {
  const relative = path.relative(rootRealpath, targetRealpath);
  if (
    relative === "" ||
    (!relative.startsWith("..") && !path.isAbsolute(relative))
  ) {
    return;
  }
  throw new ProjectReferencesError(
    `${relativePath} resolves outside the workspace`,
    [
      {
        level: "error",
        code: "outside_workspace",
        message: `${relativePath} resolves outside the workspace root.`,
        packagePath: relativePath
      }
    ]
  );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function sortPaths(paths: Array<string>): Array<string> {
  return [...paths].sort((a, b) => a.localeCompare(b));
}

function dedupe<T>(values: Array<T>): Array<T> {
  return [...new Set(values)];
}

function toPosix(value: string): string {
  return value.split(path.sep).join("/").replace(/\\/g, "/");
}
