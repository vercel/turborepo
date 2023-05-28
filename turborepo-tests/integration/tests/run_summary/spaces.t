Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd)

Kill what's running on port 8000 first, but also return 0 exit code if nothing is running on 8000
  $ PID=$(lsof -t -i:8000 2>/dev/null) && [[ -n $PID ]] && kill $PID || true

Start mock server. Note if anything fails in the test run after this,
the cleanup step won't run at the end so we have to be careful
send stdout and stderr to /dev/null and also background the server
  $ node "${TESTDIR}/mock-api.js" --port 8000 &

Run turbo. Since we have a --token (Note: remote caching will be turned on, and there isn't a way to turn it off for this test)
  $ TURBO_API=http://localhost:8000 ${TURBO} run build --experimental-space-id=myspace --token="sometokenfromcli" --team="some-team-from-cli"
Expect 3 POST requests. 
(Note: there's a 4th one for analytics, but we aren't ignoring it entirely and there isn't a way to turn off analytics)
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
