Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd)


Start mock server. Note if anything fails in the test run after this,
the cleanup step won't run at the end so we have to be careful
send stdout and stderr to /dev/null and also background the server
  $ cargo run -p vercel-api-mock & # this writes a server.pid file we can clean up after

  $ PORT=$(cat server.port)
  $ echo $PORT

Run turbo. Since we have a --token, remote caching is turned on, and there isn't a way to turn it off for this test.
  $ TURBO_API=http://localhost:$PORT ${TURBO} run build --experimental-space-id=myspace --token="sometokenfromcli" --team="some-team-from-cli" > /dev/null 2>&1

Expect 3 POST requests. There's a 4th one for analytics, but we aren't ignoring it entirely and there
isn't a way to turn off analytics from the CLI command
  $ ls post-*.json
  post-0.json
  post-1.json
  post-2.json
  post-3.json

And a PATCH request
  $ ls patch-*.json
  patch-0.json

post-0.json should be the run URL
  $ cat post-0.json | jq -r '.requestUrl'
  /v0/spaces/myspace/runs?slug=some-team-from-cli
  $ cat post-0.json | jq '.requestBody.startTime'
  [0-9]+ (re)
  $ cat post-0.json | jq '.requestBody.status'
  "running"
  $ cat post-0.json | jq '.requestBody.type'
  "TURBO"
  $ cat post-0.json | jq '.requestBody.command'
  "turbo run build"
  $ cat post-0.json | jq '.requestBody.client | keys'
  [
    "id",
    "name",
    "version"
  ]
  $ cat post-0.json | jq '.requestBody.gitBranch'
  .+ (re)
  $ cat post-0.json | jq '.requestBody.gitSha'
  .+ (re)

post-1 and post-2 should be for task summaries
  $ cat post-1.json | jq '.requestUrl'
  "/v0/spaces/myspace/runs/1234/tasks?slug=some-team-from-cli"

  $ cat post-2.json | jq '.requestUrl'
  "/v0/spaces/myspace/runs/1234/tasks?slug=some-team-from-cli"

Spot check the first task POST that all the keys were sent
  $ cat post-1.json | jq '.requestBody | keys'
  [
    "cache",
    "endTime",
    "hash",
    "key",
    "log",
    "name",
    "startTime",
    "workspace"
  ]

Patch request is pretty small so we can validate the URL and request payload together
  $ cat patch-0.json | jq
  {
    "requestUrl": "/v0/spaces/myspace/runs/1234?slug=some-team-from-cli",
    "requestBody": {
      "endTime": [0-9]+, (re)
      "status": "completed",
      "client": {
        "id": "",
        "name": "",
        "version": ""
      },
      "gitBranch": "",
      "gitSha": ""
    }
  }

Kill mock server after
  $ pid=$(cat server.pid)
  $ rm server.pid
  $ kill -9 $pid
