"""Exercise unmodified Bun and Turbo's real HTTP cache in isolated workspaces."""

import argparse
import collections
import http.server
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading
import urllib.parse


def write(root, name, contents):
    path = root / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(contents)


def run(args, cwd, env):
    result = subprocess.run(args, cwd=cwd, env=env, text=True, capture_output=True)
    if result.returncode:
        raise RuntimeError(f"{args}\n{result.stdout}\n{result.stderr}")
    return result


def initialize(root, env):
    run(["git", "init", "--quiet"], root, env)
    run(["bun", "install"], root, env)


def populate(root, consumers):
    write(root, "package.json", json.dumps({
        "name": "native-file-cache-proof", "private": True,
        "packageManager": "bun@1.4.3", "workspaces": ["packages/*"],
    }))
    write(root, ".gitignore", ".turbo\nnode_modules\n.executions\n")
    write(root, "turbo.json", json.dumps({
        "agentGuidance": False,
        "futureFlags": {"experimentalTaskCommand": True},
        "tasks": {"//#test": {
            "experimentalFiles": {"include": ["packages/*/*.test.ts"], "trace": True},
            "command": ["bun", "test", "$TURBO_FILE$"],
            "inputs": [], "outputs": [],
        }},
    }))
    write(root, "packages/shared/package.json", json.dumps({"name": "@proof/shared"}))
    write(root, "packages/shared/common.ts", "export const value = 1;\n")
    write(root, "packages/shared/exclusive.ts", "export const value = 1;\n")
    for i in range(consumers):
        write(root, f"packages/app-{i}/package.json", json.dumps({
            "name": f"@proof/app-{i}", "dependencies": {"@proof/shared": "workspace:*"},
        }))
        source = "exclusive" if i == 0 else "common"
        write(root, f"packages/app-{i}/unit.test.ts", f"""import {{ test, expect }} from 'bun:test';
import {{ appendFileSync }} from 'node:fs';
import {{ value }} from '../shared/{source}.ts';
test('validates value', () => {{
    appendFileSync('.executions', 'app-{i}\\n');
    expect(value).toBe(1);
}});
""")
    write(root, "packages/shared/private.test.ts", """import { test, expect } from 'bun:test';
import { appendFileSync } from 'node:fs';
test('private check', () => {
    appendFileSync('.executions', 'shared\\n');
    expect(2 + 2).toBe(4);
});
""")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--consumers", type=int, default=199)
    args = parser.parse_args()
    if args.consumers < 2:
        parser.error("at least two consumers are required")
    binary = str(args.binary.resolve())
    artifacts = {}
    requests = collections.Counter()

    class Cache(http.server.BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def respond(self, status, body=b"", duration=None):
            self.send_response(status)
            self.send_header("Content-Length", str(len(body)))
            if duration is not None:
                self.send_header("x-artifact-duration", duration)
            self.end_headers()
            if self.command != "HEAD":
                self.wfile.write(body)

        def read(self):
            return self.rfile.read(int(self.headers.get("Content-Length", 0)))

        def do_PUT(self):
            key = urllib.parse.urlparse(self.path).path
            artifacts[key] = (self.read(), self.headers.get("x-artifact-duration", "0"))
            requests["uploads"] += 1
            self.respond(200)

        def do_GET(self):
            key = urllib.parse.urlparse(self.path).path
            if key == "/v8/artifacts/status":
                self.respond(200, b'{"status":"enabled"}')
            elif key in artifacts:
                if self.command == "GET":
                    requests["downloads"] += 1
                self.respond(200, *artifacts[key])
            else:
                self.respond(404)

        do_HEAD = do_GET

        def do_POST(self):
            body = self.read()
            if urllib.parse.urlparse(self.path).path == "/v8/artifacts":
                hashes = json.loads(body)["hashes"]
                result = {key: ({"taskDurationMs": 0} if f"/v8/artifacts/{key}" in artifacts else None) for key in hashes}
                self.respond(200, json.dumps(result).encode())
            else:
                self.respond(200, b"{}")

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Cache)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix="turbo-native-proof-") as temporary:
            base = Path(temporary)
            env = {key: value for key, value in os.environ.items() if not key.startswith(("TURBO_", "VERCEL_ARTIFACTS_"))}
            env.update({
                "TURBO_TOKEN": "local-proof-token", "TURBO_TEAM": "local-proof-team",
                "TURBO_CONFIG_DIR_PATH": str(base / "config"),
                "TURBO_TELEMETRY_DISABLED": "1", "DO_NOT_TRACK": "1",
            })
            local = base / "agent"
            local.mkdir()
            populate(local, args.consumers)
            initialize(local, env)
            ci = base / "ci"
            shutil.copytree(local, ci, ignore=shutil.ignore_patterns(".git", "node_modules", ".turbo", ".executions"))
            initialize(ci, env)

            def execute(root, task):
                log = root / ".executions"
                log.write_text("")
                run([binary, "run", task, "--skip-infer", "--cache=remote:rw", "--output-logs=errors-only", "--concurrency=10", f"--api=http://127.0.0.1:{server.server_port}"], root, env)
                return collections.Counter(log.read_text().splitlines())

            focused = "test@packages%2Fapp-0%2Funit.test.ts"
            assert execute(local, focused) == {"app-0": 1}
            assert requests["uploads"] == 1, requests
            expected = collections.Counter({f"app-{i}": 1 for i in range(1, args.consumers)})
            expected["shared"] = 1
            assert execute(ci, "test") == expected
            assert requests["downloads"] >= 1, requests
            assert execute(local, "test@packages%2Fapp-1%2Funit.test.ts") == {}
            with (ci / "packages/shared/private.test.ts").open("a") as changed:
                changed.write("// changed private check\n")
            assert execute(ci, "test") == {"shared": 1}
            with (ci / "packages/shared/exclusive.ts").open("a") as changed:
                changed.write("// changed source module\n")
            assert execute(ci, "test") == {"app-0": 1}
            print(json.dumps({
                "packages": args.consumers + 1,
                "focused_run_executed": 1,
                "separate_checkout_full_run_executed": args.consumers,
                "focused_run_after_ci_executed": 0,
                "private_test_edit_executed": 1,
                "exclusive_source_edit_executed": 1,
                "cache_transport": "Turbo HTTP API against loopback test server",
                "http_requests": dict(requests),
                "bun_version": run(["bun", "--version"], local, env).stdout.strip(),
            }, indent=2))
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


if __name__ == "__main__":
    main()
