"use client";

import {
  startTransition,
  useEffect,
  useEffectEvent,
  useRef,
  useState
} from "react";

import {
  type FactoryImageBuild,
  type FactoryImageBuildStatus,
  type FactoryImageView,
  FACTORY_IMAGE_REBUILD_ACTION
} from "../agent/lib/factory-image-types";
import { Button } from "../components/ui/button";

const POLL_INTERVAL_MS = 15_000;

const BUILD_STATE_CLASS: Record<FactoryImageBuildStatus, string> = {
  building: "running",
  cancelled: "failed",
  failed: "failed",
  publishing: "running",
  queued: "waiting",
  ready: "completed"
};

function formatTimestamp(value: string): string {
  return `${new Intl.DateTimeFormat("en-US", {
    dateStyle: "medium",
    timeStyle: "short",
    timeZone: "UTC"
  }).format(new Date(value))} UTC`;
}

function BuildCard({
  build,
  log
}: {
  readonly build: FactoryImageBuild;
  readonly log?: string;
}) {
  return (
    <li
      className={`min-w-0 border-t border-border pt-4 ${log ? "col-span-full" : ""}`}
    >
      <div className="flex items-center gap-2">
        <span
          className={`flex items-center gap-2 font-mono text-xs capitalize ${BUILD_STATE_CLASS[build.status] === "running" || BUILD_STATE_CLASS[build.status] === "waiting" ? "text-warning" : BUILD_STATE_CLASS[build.status] === "completed" ? "text-success" : "text-destructive"}`}
        >
          <span
            className="size-[7px] shrink-0 rounded-full bg-current"
            aria-hidden="true"
          />
          <span>{build.status}</span>
        </span>
        <strong className="overflow-hidden text-ellipsis whitespace-nowrap font-mono text-xs font-medium">
          {build.commit.slice(0, 7)}
        </strong>
      </div>
      <dl
        className={`ml-[15px] mt-4 grid gap-2 ${log ? "grid-cols-3 max-[520px]:grid-cols-1" : ""}`}
      >
        <div>
          <dt className="text-xs text-muted-foreground">Phase</dt>
          <dd className="mt-1 overflow-hidden text-ellipsis whitespace-nowrap font-mono text-xs text-foreground">
            {build.phase ?? "unknown"}
          </dd>
        </div>
        <div>
          <dt className="text-xs text-muted-foreground">Trigger</dt>
          <dd className="mt-1 overflow-hidden text-ellipsis whitespace-nowrap font-mono text-xs text-foreground">
            {build.trigger}
          </dd>
        </div>
        <div>
          <dt className="text-xs text-muted-foreground">Updated</dt>
          <dd className="mt-1 overflow-hidden text-ellipsis whitespace-nowrap font-mono text-xs text-foreground">
            <time dateTime={build.updatedAt}>
              {formatTimestamp(build.updatedAt)}
            </time>
          </dd>
        </div>
      </dl>
      {build.message ? (
        <p className="mt-6 max-w-[68ch] text-muted-foreground">
          {build.message}
        </p>
      ) : null}
      {log ? (
        <pre
          className="mt-4 max-h-80 overflow-auto rounded-md border border-border bg-secondary p-4 font-mono text-xs leading-[1.6] whitespace-pre-wrap"
          aria-label="Live factory image build log"
        >
          {log}
        </pre>
      ) : null}
    </li>
  );
}

export function FactoryImage({
  initialView
}: {
  readonly initialView: FactoryImageView;
}) {
  const [view, setView] = useState(initialView);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const controller = useRef<AbortController | null>(null);

  async function refresh() {
    if (controller.current) return;
    const next = new AbortController();
    controller.current = next;
    try {
      const response = await fetch("/api/factory-image", {
        cache: "no-store",
        signal: next.signal
      });
      if (!response.ok) throw new Error("Could not read the factory image.");
      const value = (await response.json()) as FactoryImageView;
      startTransition(() => {
        setView(value);
        setError(null);
      });
    } catch (failure) {
      if (!(failure instanceof DOMException && failure.name === "AbortError")) {
        startTransition(() =>
          setError(
            failure instanceof Error
              ? failure.message
              : "Could not read the factory image."
          )
        );
      }
    } finally {
      controller.current = null;
    }
  }
  const poll = useEffectEvent(refresh);

  useEffect(() => {
    const timer = window.setInterval(() => {
      if (document.visibilityState === "visible") void poll();
    }, POLL_INTERVAL_MS);
    return () => {
      controller.current?.abort();
      window.clearInterval(timer);
    };
  }, []);

  async function rebuild() {
    setBusy(true);
    setError(null);
    try {
      const response = await fetch("/api/factory-image", {
        body: "{}",
        headers: {
          "content-type": "application/json",
          "x-operator-action": FACTORY_IMAGE_REBUILD_ACTION
        },
        method: "POST"
      });
      if (!response.ok) throw new Error("Could not start the image build.");
      await refresh();
    } catch (failure) {
      setError(
        failure instanceof Error
          ? failure.message
          : "Could not start the image build."
      );
    } finally {
      setBusy(false);
    }
  }

  const { pointer } = view;
  const stale = pointer !== null && pointer.fingerprint !== view.fingerprint;

  return (
    <section
      className="grid grid-cols-[minmax(220px,4fr)_minmax(0,8fr)] gap-x-16 border-b border-border py-[clamp(64px,10vw,120px)] max-[720px]:block max-[720px]:py-16"
      aria-labelledby="factory-image-title"
    >
      <div className="contents max-[720px]:flex max-[720px]:items-start max-[720px]:justify-between max-[720px]:gap-4 max-[520px]:flex-col-reverse">
        <div className="col-start-1 row-span-4 max-[720px]:block">
          <h2
            className="text-[clamp(1.5rem,3vw,2rem)] leading-tight font-semibold tracking-[-0.04em]"
            id="factory-image-title"
          >
            Factory image
          </h2>
        </div>
        <span className="col-start-2 self-start justify-self-start font-mono text-xs tracking-[0.02em] text-muted-foreground">
          ON MERGE TO MAIN
        </span>
      </div>

      <p className="col-start-2 mt-6 max-w-[68ch] text-muted-foreground max-[720px]:mt-6">
        Every push to <code>main</code> rebuilds the sandbox snapshot each agent
        boots from: a Turborepo checkout at that commit plus the Rust, protoc,
        Cap&apos;n Proto, Zig, and pnpm toolchain, installed dependencies, and a
        warm Cargo target. Rapid merges cancel the in-flight build so only the
        newest revision is published.
      </p>

      {view.configured ? null : (
        <div
          className="col-start-2 mt-6 border-l-2 border-warning bg-muted px-3.5 py-3 text-sm"
          role="status"
        >
          Connect a private Vercel Blob store to enable factory image builds.
        </div>
      )}
      {error ? (
        <div
          className="col-start-2 mt-6 border-l-2 border-destructive bg-muted px-3.5 py-3 text-sm"
          role="alert"
        >
          {error}
        </div>
      ) : null}
      {stale ? (
        <div
          className="col-start-2 mt-6 border-l-2 border-warning bg-muted px-3.5 py-3 text-sm"
          role="status"
        >
          The published image was built for an older toolchain. The next merge
          rebuilds it.
        </div>
      ) : null}

      <dl className="col-start-2 my-10 grid grid-cols-3 gap-6 max-[520px]:grid-cols-1 max-[520px]:gap-4">
        <div className="min-w-0 border-t border-border pt-3">
          <dt className="text-xs text-muted-foreground">Commit</dt>
          <dd className="mt-1.5 text-sm">
            <code>{pointer ? pointer.commit.slice(0, 7) : "none"}</code>
          </dd>
        </div>
        <div className="min-w-0 border-t border-border pt-3">
          <dt className="text-xs text-muted-foreground">Snapshot</dt>
          <dd className="mt-1.5 text-sm">
            <code>{pointer ? pointer.snapshotId : "not published"}</code>
          </dd>
        </div>
        <div className="min-w-0 border-t border-border pt-3">
          <dt className="text-xs text-muted-foreground">Published</dt>
          <dd className="mt-1.5 text-sm">
            {pointer ? formatTimestamp(pointer.publishedAt) : "never"}
          </dd>
        </div>
        <div className="min-w-0 border-t border-border pt-3">
          <dt className="text-xs text-muted-foreground">Toolchain</dt>
          <dd className="mt-1.5 text-sm">
            <code>{view.fingerprint}</code>
          </dd>
        </div>
      </dl>

      {pointer?.warnings && pointer.warnings.length > 0 ? (
        <div
          className="col-start-2 mt-6 border-l-2 border-warning bg-muted px-3.5 py-3 text-sm"
          role="status"
        >
          {pointer.warnings.join(" ")}
        </div>
      ) : null}

      <div className="col-start-2 border-t border-border pt-8">
        <div className="flex flex-wrap items-center gap-2.5">
          <Button
            disabled={busy || !view.configured}
            onClick={() => void rebuild()}
            type="button"
          >
            {busy ? "Starting…" : "Rebuild from main"}
          </Button>
        </div>
      </div>

      <div className="col-start-2 mt-12 flex items-baseline justify-between">
        <h3 className="text-pretty text-base font-semibold tracking-[-0.02em]">
          Recent builds
        </h3>
        <span className="text-xs text-muted-foreground">
          {view.builds.length} builds
        </span>
      </div>
      {view.builds.length > 0 ? (
        <ul className="col-start-2 mt-5 grid list-none grid-cols-4 gap-6 p-0 max-[980px]:grid-cols-2 max-[520px]:grid-cols-1">
          {view.builds.map((build) => (
            <BuildCard
              build={build}
              key={build.id}
              log={view.logs?.[build.id]}
            />
          ))}
        </ul>
      ) : (
        <p className="col-start-2 mt-4 border-y border-border py-8 text-center text-sm text-muted-foreground">
          No factory image builds are recorded.
        </p>
      )}
    </section>
  );
}
