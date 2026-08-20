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

function BuildCard({ build }: { readonly build: FactoryImageBuild }) {
  return (
    <li className="sandboxCard">
      <div>
        <span
          className={`runState runState-${BUILD_STATE_CLASS[build.status]}`}
        >
          <span className="statusDot" aria-hidden="true" />
          <span>{build.status}</span>
        </span>
        <strong>{build.commit.slice(0, 7)}</strong>
      </div>
      <dl>
        <div>
          <dt>Phase</dt>
          <dd>{build.phase ?? "unknown"}</dd>
        </div>
        <div>
          <dt>Trigger</dt>
          <dd>{build.trigger}</dd>
        </div>
        <div>
          <dt>Updated</dt>
          <dd>
            <time dateTime={build.updatedAt}>
              {formatTimestamp(build.updatedAt)}
            </time>
          </dd>
        </div>
      </dl>
      {build.message ? <p className="description">{build.message}</p> : null}
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
    <section className="operation" aria-labelledby="factory-image-title">
      <div className="operationHeader">
        <div>
          <h2 id="factory-image-title">Factory image</h2>
        </div>
        <span className="schedule">ON MERGE TO MAIN</span>
      </div>

      <p className="description">
        Every push to <code>main</code> rebuilds the sandbox snapshot each agent
        boots from: a Turborepo checkout at that commit plus the Rust, protoc,
        Cap&apos;n Proto, Zig, and pnpm toolchain, installed dependencies, and a
        warm Cargo target. Rapid merges cancel the in-flight build so only the
        newest revision is published.
      </p>

      {view.configured ? null : (
        <div className="registryNotice" role="status">
          Connect a private Vercel Blob store to enable factory image builds.
        </div>
      )}
      {error ? (
        <div className="registryNotice registryNotice-error" role="alert">
          {error}
        </div>
      ) : null}
      {stale ? (
        <div className="registryNotice" role="status">
          The published image was built for an older toolchain. The next merge
          rebuilds it.
        </div>
      ) : null}

      <dl className="facts">
        <div>
          <dt>Commit</dt>
          <dd>
            <code>{pointer ? pointer.commit.slice(0, 7) : "none"}</code>
          </dd>
        </div>
        <div>
          <dt>Snapshot</dt>
          <dd>
            <code>{pointer ? pointer.snapshotId : "not published"}</code>
          </dd>
        </div>
        <div>
          <dt>Published</dt>
          <dd>{pointer ? formatTimestamp(pointer.publishedAt) : "never"}</dd>
        </div>
        <div>
          <dt>Toolchain</dt>
          <dd>
            <code>{view.fingerprint}</code>
          </dd>
        </div>
      </dl>

      {pointer?.warnings && pointer.warnings.length > 0 ? (
        <div className="registryNotice" role="status">
          {pointer.warnings.join(" ")}
        </div>
      ) : null}

      <div className="controls">
        <div className="actions">
          <Button
            disabled={busy || !view.configured}
            onClick={() => void rebuild()}
            type="button"
          >
            {busy ? "Starting…" : "Rebuild from main"}
          </Button>
        </div>
      </div>

      <div className="sandboxInventoryHeader">
        <h3>Recent builds</h3>
        <span>{view.builds.length} builds</span>
      </div>
      {view.builds.length > 0 ? (
        <ul className="sandboxGrid">
          {view.builds.map((build) => (
            <BuildCard build={build} key={build.id} />
          ))}
        </ul>
      ) : (
        <p className="emptySandboxes">No factory image builds are recorded.</p>
      )}
    </section>
  );
}
