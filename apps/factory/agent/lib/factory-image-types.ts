/**
 * Ledger types and state machine for factory image builds.
 *
 * Every transition is a pure function over the ledger so the
 * supersede-and-cancel rules that keep only the newest `main` revision
 * winning can be unit tested without touching Vercel Blob.
 * `factory-image-registry.ts` wraps these in compare-and-swap writes.
 */

export const FACTORY_IMAGE_BUILD_STATUSES = [
  "queued",
  "building",
  "publishing",
  "ready",
  "cancelled",
  "failed"
] as const;

export type FactoryImageBuildStatus =
  (typeof FACTORY_IMAGE_BUILD_STATUSES)[number];

export type FactoryImageTrigger = "operator" | "webhook";

/** Builds in these states still own a sandbox and can be superseded. */
const ACTIVE_STATUSES = new Set<FactoryImageBuildStatus>([
  "queued",
  "building",
  "publishing"
]);

/** Builds retained in the ledger, newest first. */
const MAX_BUILDS = 20;

/**
 * How long a build may go without reporting progress before another
 * claim for the same revision replaces it instead of deduplicating
 * against it. Without this a build whose deployment vanished mid-flight
 * would wedge that revision forever.
 */
const STALE_BUILD_MS = 15 * 60 * 1000;

export interface FactoryImageBuild {
  readonly commit: string;
  readonly createdAt: string;
  readonly fingerprint: string;
  readonly finishedAt?: string;
  readonly id: string;
  /** Latest human-readable detail: a failure reason or a warning count. */
  readonly message?: string;
  readonly phase?: string;
  readonly ref: string;
  readonly sandboxName: string;
  readonly snapshotId?: string;
  readonly status: FactoryImageBuildStatus;
  /** Id of the build that cancelled this one. */
  readonly supersededBy?: string;
  readonly trigger: FactoryImageTrigger;
  readonly updatedAt: string;
}

export interface FactoryImagePointer {
  readonly buildId: string;
  readonly commit: string;
  readonly fingerprint: string;
  readonly publishedAt: string;
  readonly sandboxName: string;
  readonly snapshotId: string;
  readonly tools?: Readonly<Record<string, string>>;
  readonly warmBuild: boolean;
  readonly warnings?: readonly string[];
}

export interface FactoryImageState {
  readonly builds: readonly FactoryImageBuild[];
  readonly pointer: FactoryImagePointer | null;
}

export const EMPTY_FACTORY_IMAGE_STATE: FactoryImageState = {
  builds: [],
  pointer: null
};

export interface FactoryImageClaimInput {
  readonly buildId: string;
  readonly commit: string;
  readonly fingerprint: string;
  readonly now: string;
  readonly ref: string;
  readonly sandboxName: string;
  readonly trigger: FactoryImageTrigger;
}

export type FactoryImageClaim =
  /** A build was claimed; `superseded` builds must be cancelled. */
  | {
      readonly build: FactoryImageBuild;
      readonly kind: "claimed";
      readonly state: FactoryImageState;
      readonly superseded: readonly FactoryImageBuild[];
    }
  /** The published image already matches this revision and toolchain. */
  | { readonly kind: "current"; readonly pointer: FactoryImagePointer }
  /** A live build already covers this revision (webhook redelivery). */
  | { readonly build: FactoryImageBuild; readonly kind: "in-progress" };

export function isFactoryImageBuildActive(build: FactoryImageBuild): boolean {
  return ACTIVE_STATUSES.has(build.status);
}

export function activeFactoryImageBuilds(
  state: FactoryImageState
): FactoryImageBuild[] {
  return state.builds.filter(isFactoryImageBuildActive);
}

/** An active build that stopped reporting progress long enough ago. */
export function isStaleFactoryImageBuild(
  build: FactoryImageBuild,
  now: string
): boolean {
  const updated = Date.parse(build.updatedAt);
  const current = Date.parse(now);
  if (Number.isNaN(updated) || Number.isNaN(current)) return false;
  return current - updated > STALE_BUILD_MS;
}

export function findFactoryImageBuild(
  state: FactoryImageState,
  buildId: string
): FactoryImageBuild | null {
  return state.builds.find((build) => build.id === buildId) ?? null;
}

/**
 * Claims the ledger for one revision.
 *
 * A rapid series of merges therefore leaves exactly one live build: each
 * claim cancels every build still in flight and records which build
 * replaced it, so the caller can delete their sandboxes.
 */
export function claimFactoryImageBuild(
  state: FactoryImageState,
  input: FactoryImageClaimInput
): FactoryImageClaim {
  if (
    state.pointer !== null &&
    state.pointer.commit === input.commit &&
    state.pointer.fingerprint === input.fingerprint
  ) {
    return { kind: "current", pointer: state.pointer };
  }

  const existing = activeFactoryImageBuilds(state).find(
    (build) =>
      build.commit === input.commit && build.fingerprint === input.fingerprint
  );
  if (existing && !isStaleFactoryImageBuild(existing, input.now)) {
    return { build: existing, kind: "in-progress" };
  }

  const build: FactoryImageBuild = {
    commit: input.commit,
    createdAt: input.now,
    fingerprint: input.fingerprint,
    id: input.buildId,
    phase: "queued",
    ref: input.ref,
    sandboxName: input.sandboxName,
    status: "queued",
    trigger: input.trigger,
    updatedAt: input.now
  };
  const superseded = activeFactoryImageBuilds(state).map(
    (candidate): FactoryImageBuild => ({
      ...candidate,
      finishedAt: input.now,
      message: `Superseded by ${input.commit.slice(0, 7)}.`,
      status: "cancelled",
      supersededBy: build.id,
      updatedAt: input.now
    })
  );
  const cancelled = new Map(superseded.map((entry) => [entry.id, entry]));
  return {
    build,
    kind: "claimed",
    state: {
      builds: [
        build,
        ...state.builds.map(
          (candidate) => cancelled.get(candidate.id) ?? candidate
        )
      ].slice(0, MAX_BUILDS),
      pointer: state.pointer
    },
    superseded
  };
}

export type FactoryImageBuildChanges = Partial<
  Pick<
    FactoryImageBuild,
    "finishedAt" | "message" | "phase" | "snapshotId" | "status"
  >
>;

export function beginFactoryImageProvisioning(
  state: FactoryImageState,
  buildId: string,
  now: string
): {
  readonly build: FactoryImageBuild | null;
  readonly state: FactoryImageState;
} {
  const build = findFactoryImageBuild(state, buildId);
  if (
    build === null ||
    (build.status !== "queued" &&
      !(
        build.status === "building" &&
        build.phase === "starting" &&
        isStaleFactoryImageBuild(build, now)
      ))
  ) {
    return { build: null, state };
  }
  const next = updateFactoryImageBuild(
    state,
    buildId,
    { phase: "starting", status: "building" },
    now
  );
  return { build: findFactoryImageBuild(next, buildId), state: next };
}

export function beginFactoryImagePublication(
  state: FactoryImageState,
  buildId: string,
  now: string
): {
  readonly build: FactoryImageBuild | null;
  readonly state: FactoryImageState;
} {
  const build = findFactoryImageBuild(state, buildId);
  if (
    build === null ||
    (build.status !== "building" &&
      !(build.status === "publishing" && isStaleFactoryImageBuild(build, now)))
  ) {
    return { build: null, state };
  }
  const next = updateFactoryImageBuild(
    state,
    buildId,
    { phase: "snapshotting", status: "publishing" },
    now
  );
  return { build: findFactoryImageBuild(next, buildId), state: next };
}

/**
 * Applies progress to one build, ignoring updates to builds that already
 * reached a terminal state. That single rule is what stops a superseded
 * build from resurrecting itself when a step that was already in flight
 * reports back.
 */
export function updateFactoryImageBuild(
  state: FactoryImageState,
  buildId: string,
  changes: FactoryImageBuildChanges,
  now: string
): FactoryImageState {
  const current = findFactoryImageBuild(state, buildId);
  if (current === null || !isFactoryImageBuildActive(current)) return state;
  return {
    builds: state.builds.map((build) =>
      build.id === buildId
        ? { ...build, ...changes, id: buildId, updatedAt: now }
        : build
    ),
    pointer: state.pointer
  };
}

export interface FactoryImagePublishInput {
  readonly now: string;
  readonly snapshotId: string;
  readonly tools?: Readonly<Record<string, string>>;
  readonly warmBuild: boolean;
  readonly warnings?: readonly string[];
}

/**
 * Publishes one build's snapshot as the current factory image. Refuses
 * when the build is no longer active, so a build that lost the race to a
 * newer merge can never overwrite the newer pointer.
 */
export function publishFactoryImagePointer(
  state: FactoryImageState,
  buildId: string,
  input: FactoryImagePublishInput
): { readonly published: boolean; readonly state: FactoryImageState } {
  const build = findFactoryImageBuild(state, buildId);
  if (build === null || !isFactoryImageBuildActive(build)) {
    return { published: false, state };
  }
  const pointer: FactoryImagePointer = {
    buildId,
    commit: build.commit,
    fingerprint: build.fingerprint,
    publishedAt: input.now,
    sandboxName: build.sandboxName,
    snapshotId: input.snapshotId,
    tools: input.tools,
    warmBuild: input.warmBuild,
    warnings: input.warnings
  };
  return {
    published: true,
    state: {
      builds: state.builds.map((candidate) =>
        candidate.id === buildId
          ? {
              ...candidate,
              finishedAt: input.now,
              message:
                input.warnings === undefined || input.warnings.length === 0
                  ? undefined
                  : `${input.warnings.length} warning(s).`,
              phase: "done",
              snapshotId: input.snapshotId,
              status: "ready",
              updatedAt: input.now
            }
          : candidate
      ),
      pointer
    }
  };
}

export function isFactoryImageBuild(
  value: unknown
): value is FactoryImageBuild {
  if (typeof value !== "object" || value === null) return false;
  const build = value as Record<string, unknown>;
  return (
    typeof build.commit === "string" &&
    typeof build.createdAt === "string" &&
    typeof build.fingerprint === "string" &&
    typeof build.id === "string" &&
    typeof build.ref === "string" &&
    typeof build.sandboxName === "string" &&
    FACTORY_IMAGE_BUILD_STATUSES.some((status) => status === build.status) &&
    (build.trigger === "operator" || build.trigger === "webhook") &&
    typeof build.updatedAt === "string"
  );
}

export function isFactoryImagePointer(
  value: unknown
): value is FactoryImagePointer {
  if (typeof value !== "object" || value === null) return false;
  const pointer = value as Record<string, unknown>;
  return (
    typeof pointer.buildId === "string" &&
    typeof pointer.commit === "string" &&
    typeof pointer.fingerprint === "string" &&
    typeof pointer.publishedAt === "string" &&
    typeof pointer.sandboxName === "string" &&
    typeof pointer.snapshotId === "string" &&
    typeof pointer.warmBuild === "boolean"
  );
}

export function parseFactoryImageState(value: unknown): FactoryImageState {
  if (typeof value !== "object" || value === null) {
    return EMPTY_FACTORY_IMAGE_STATE;
  }
  const state = value as Record<string, unknown>;
  return {
    builds: Array.isArray(state.builds)
      ? state.builds.filter(isFactoryImageBuild)
      : [],
    pointer: isFactoryImagePointer(state.pointer) ? state.pointer : null
  };
}

/** Ledger projection served to the operator dashboard. */
export interface FactoryImageView {
  readonly builds: readonly FactoryImageBuild[];
  readonly configured: boolean;
  /** Toolchain fingerprint this deployment expects. */
  readonly fingerprint: string;
  /** Recent output for active builds, keyed by build id. */
  readonly logs?: Readonly<Record<string, string>>;
  readonly pointer: FactoryImagePointer | null;
}

/** Sent as `x-operator-action` when an operator rebuilds the image. */
export const FACTORY_IMAGE_REBUILD_ACTION = "rebuild-factory-image";

/** Sandbox name for one build, readable in the sandbox inventory. */
export function factoryImageSandboxName(
  commit: string,
  buildId: string
): string {
  return `factory-image-${commit.slice(0, 12)}-${buildId.slice(0, 8)}`;
}

export const FACTORY_IMAGE_SANDBOX_PREFIX = "factory-image-";
