window.BENCHMARK_DATA = {
  "lastUpdate": 1655166953024,
  "repoUrl": "https://github.com/vercel/turborepo",
  "entries": {
    "Linux Benchmark": [
      {
        "commit": {
          "author": {
            "email": "greg.soltis@vercel.com",
            "name": "Greg Soltis",
            "username": "gsoltis"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "4d42fa555ad2a6dc5b1c33fb8ee1bb6c4ae9299d",
          "message": "Large Monorepo Benchmark (#740)\n\nRuns a set of benchmarks against a large monorepo example. \n\nBenchmarks include:\n * A clean, zero-state build* of the monorepo\n * Building the unchanged monorepo with a full cache\n * Building the monorepo with a source code edit on top of a previously-built cache\n * Building the monorepo with a dependency graph edit on top of a previously-build cache\n\n*: Note that we only do this once, and with `concurrency` set to `1`. A full concurrent build currently OOMs",
          "timestamp": "2022-02-15T22:29:29Z",
          "tree_id": "339dab36b2fdf6d11d1e9c4d4681e9ab5d927612",
          "url": "https://github.com/vercel/turborepo/commit/4d42fa555ad2a6dc5b1c33fb8ee1bb6c4ae9299d"
        },
        "date": 1644965502570,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 154258,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 10261.6,
            "unit": "ms",
            "range": "408"
          },
          {
            "name": "Cached Build - source code change",
            "value": 35279.6,
            "unit": "ms",
            "range": "10198"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 35709.4,
            "unit": "ms",
            "range": "7464"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "3476570+ivoilic@users.noreply.github.com",
            "name": "Ivo Ilić",
            "username": "ivoilic"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "eccacc9dc0f239b278a204ba1344fe9077015114",
          "message": "Don't run build & test on changes to the docs (#737)\n\nCo-authored-by: Jared Palmer <jared@jaredpalmer.com>",
          "timestamp": "2022-02-16T10:57:20-05:00",
          "tree_id": "cfec4cb244bd97f0ae6a1a1fb09a125c0a89d356",
          "url": "https://github.com/vercel/turborepo/commit/eccacc9dc0f239b278a204ba1344fe9077015114"
        },
        "date": 1645028529133,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 170578.4,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 10771.4,
            "unit": "ms",
            "range": "512"
          },
          {
            "name": "Cached Build - source code change",
            "value": 39879.6,
            "unit": "ms",
            "range": "11987"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 39542.4,
            "unit": "ms",
            "range": "9163"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "jared@jaredpalmer.com",
            "name": "Jared Palmer",
            "username": "jaredpalmer"
          },
          "committer": {
            "email": "jared@jaredpalmer.com",
            "name": "Jared Palmer",
            "username": "jaredpalmer"
          },
          "distinct": true,
          "id": "dca8f9fb4d916d48724fb6d3f85380bd3cf0b664",
          "message": "Update showcase",
          "timestamp": "2022-02-16T11:47:33-05:00",
          "tree_id": "d39610e8996a6fc3a1930735d9f90a85493d2106",
          "url": "https://github.com/vercel/turborepo/commit/dca8f9fb4d916d48724fb6d3f85380bd3cf0b664"
        },
        "date": 1645031501574,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 166916.2,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 10755,
            "unit": "ms",
            "range": "511"
          },
          {
            "name": "Cached Build - source code change",
            "value": 40386.2,
            "unit": "ms",
            "range": "9729"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 40143.8,
            "unit": "ms",
            "range": "9206"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "jared@jaredpalmer.com",
            "name": "Jared Palmer",
            "username": "jaredpalmer"
          },
          "committer": {
            "email": "jared@jaredpalmer.com",
            "name": "Jared Palmer",
            "username": "jaredpalmer"
          },
          "distinct": true,
          "id": "1251457dddaf977a45ed56fffc1b35feee2f7b41",
          "message": "Fix alt tag",
          "timestamp": "2022-02-16T11:47:53-05:00",
          "tree_id": "5c5dd46c1d70b401124cfd824c800e921ca2ebfa",
          "url": "https://github.com/vercel/turborepo/commit/1251457dddaf977a45ed56fffc1b35feee2f7b41"
        },
        "date": 1645031687944,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 191019.6,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 11162.4,
            "unit": "ms",
            "range": "307"
          },
          {
            "name": "Cached Build - source code change",
            "value": 44686.4,
            "unit": "ms",
            "range": "12393"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 43798.6,
            "unit": "ms",
            "range": "12407"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Greg Soltis",
            "username": "gsoltis",
            "email": "greg.soltis@vercel.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "09223bf43e4f3668bfd4f9b8a9599c836fbd6692",
          "message": "Start splitting the Run command up (#752)\n\nThis PR makes a few changes to `Run`. The intention is to separate out a few distinct pieces:\n * The complete graph, inferred from the the filesystem and pipeline configuration. This should be reusable across multiple runs on the same filesystem / pipeline\n * Run specific configuration: what tasks to run, whether to include dependencies, etc.\n * The mechanics of how to execute the run. In parallel? Cache the output? etc.\n\nReview note: it may be easier to go commit by commit.",
          "timestamp": "2022-02-17T17:34:44Z",
          "url": "https://github.com/vercel/turborepo/commit/09223bf43e4f3668bfd4f9b8a9599c836fbd6692"
        },
        "date": 1645144119611,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 120823,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 9962.6,
            "unit": "ms",
            "range": "460"
          },
          {
            "name": "Cached Build - source code change",
            "value": 30461.6,
            "unit": "ms",
            "range": "8714"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 33389.6,
            "unit": "ms",
            "range": "8642"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Weyert de Boer",
            "username": "weyert",
            "email": "weyert@gmail.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "4b28b180dda7f6646b5d603ef5c5f534a82219ca",
          "message": "feat: add support for JSON with comments `turbo.json` file (#745)\n\nAllow the `turbo.json` file to contain comments\n\nMakes it easier to explain the pipeline when they are quite large :)\nSo that future @weyert and his colleagues know what's going on!\n\nFixes #644\n\nCo-authored-by: tapico-weyert <70971917+tapico-weyert@users.noreply.github.com>",
          "timestamp": "2022-02-18T20:49:13Z",
          "url": "https://github.com/vercel/turborepo/commit/4b28b180dda7f6646b5d603ef5c5f534a82219ca"
        },
        "date": 1645230854683,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 153591.8,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 10734.6,
            "unit": "ms",
            "range": "250"
          },
          {
            "name": "Cached Build - source code change",
            "value": 36974.8,
            "unit": "ms",
            "range": "10113"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 37224,
            "unit": "ms",
            "range": "8635"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Shu Ding",
            "username": "shuding",
            "email": "g@shud.in"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "9ee851eadeb8531bcd8cd6c5706d7520a91c0e2f",
          "message": "Upgrade Nextra (#760)",
          "timestamp": "2022-02-19T20:27:34Z",
          "url": "https://github.com/vercel/turborepo/commit/9ee851eadeb8531bcd8cd6c5706d7520a91c0e2f"
        },
        "date": 1645317111325,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 122148.6,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 10002.8,
            "unit": "ms",
            "range": "491"
          },
          {
            "name": "Cached Build - source code change",
            "value": 32798.8,
            "unit": "ms",
            "range": "10473"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 30868.6,
            "unit": "ms",
            "range": "6633"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Shu Ding",
            "username": "shuding",
            "email": "g@shud.in"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "9ee851eadeb8531bcd8cd6c5706d7520a91c0e2f",
          "message": "Upgrade Nextra (#760)",
          "timestamp": "2022-02-19T20:27:34Z",
          "url": "https://github.com/vercel/turborepo/commit/9ee851eadeb8531bcd8cd6c5706d7520a91c0e2f"
        },
        "date": 1645403737305,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 168745.6,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 11075.4,
            "unit": "ms",
            "range": "514"
          },
          {
            "name": "Cached Build - source code change",
            "value": 40978,
            "unit": "ms",
            "range": "15815"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 41055.2,
            "unit": "ms",
            "range": "14146"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Shu Ding",
            "username": "shuding",
            "email": "g@shud.in"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "9ee851eadeb8531bcd8cd6c5706d7520a91c0e2f",
          "message": "Upgrade Nextra (#760)",
          "timestamp": "2022-02-19T20:27:34Z",
          "url": "https://github.com/vercel/turborepo/commit/9ee851eadeb8531bcd8cd6c5706d7520a91c0e2f"
        },
        "date": 1645489986321,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 150298,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 10512.8,
            "unit": "ms",
            "range": "110"
          },
          {
            "name": "Cached Build - source code change",
            "value": 39271,
            "unit": "ms",
            "range": "9452"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 36244.6,
            "unit": "ms",
            "range": "7669"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Shu Ding",
            "username": "shuding",
            "email": "g@shud.in"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "9ee851eadeb8531bcd8cd6c5706d7520a91c0e2f",
          "message": "Upgrade Nextra (#760)",
          "timestamp": "2022-02-19T20:27:34Z",
          "url": "https://github.com/vercel/turborepo/commit/9ee851eadeb8531bcd8cd6c5706d7520a91c0e2f"
        },
        "date": 1645576232404,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 120646,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 9951.2,
            "unit": "ms",
            "range": "425"
          },
          {
            "name": "Cached Build - source code change",
            "value": 30780,
            "unit": "ms",
            "range": "8205"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 30025.8,
            "unit": "ms",
            "range": "7887"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Shu Ding",
            "username": "shuding",
            "email": "g@shud.in"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "9ee851eadeb8531bcd8cd6c5706d7520a91c0e2f",
          "message": "Upgrade Nextra (#760)",
          "timestamp": "2022-02-19T20:27:34Z",
          "url": "https://github.com/vercel/turborepo/commit/9ee851eadeb8531bcd8cd6c5706d7520a91c0e2f"
        },
        "date": 1645662616257,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 126210.8,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 10014.8,
            "unit": "ms",
            "range": "567"
          },
          {
            "name": "Cached Build - source code change",
            "value": 31749,
            "unit": "ms",
            "range": "8866"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 31592.4,
            "unit": "ms",
            "range": "7314"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Jared Palmer",
            "username": "jaredpalmer",
            "email": "jared@jaredpalmer.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "996fd3b015d47dcc7581a0921b0e936ba18aa33d",
          "message": "Switch default api url to https://vercel.com/api by default (#776)",
          "timestamp": "2022-02-24T20:40:30Z",
          "url": "https://github.com/vercel/turborepo/commit/996fd3b015d47dcc7581a0921b0e936ba18aa33d"
        },
        "date": 1645749182338,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 148170.2,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 10691.8,
            "unit": "ms",
            "range": "508"
          },
          {
            "name": "Cached Build - source code change",
            "value": 36324.6,
            "unit": "ms",
            "range": "10343"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 36783.4,
            "unit": "ms",
            "range": "11136"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Jared Palmer",
            "username": "jaredpalmer",
            "email": "jared@jaredpalmer.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "3f4bd923c98497bcb059448bc745b3cea33373dc",
          "message": "Change \"Bootstrapping\" to \"Creating\" in `create-turbo` (#780)",
          "timestamp": "2022-02-25T18:40:27Z",
          "url": "https://github.com/vercel/turborepo/commit/3f4bd923c98497bcb059448bc745b3cea33373dc"
        },
        "date": 1645835972767,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 204765.8,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 11242,
            "unit": "ms",
            "range": "520"
          },
          {
            "name": "Cached Build - source code change",
            "value": 48751,
            "unit": "ms",
            "range": "12671"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 46848.4,
            "unit": "ms",
            "range": "10762"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "kokoaj",
            "username": "kokiebisu",
            "email": "43525282+kokiebisu@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "ca5a2284d60134096ffcccbd4fd4a9655c8911f4",
          "message": "document fixes for the cache section (#782)",
          "timestamp": "2022-02-26T21:22:54Z",
          "url": "https://github.com/vercel/turborepo/commit/ca5a2284d60134096ffcccbd4fd4a9655c8911f4"
        },
        "date": 1645922262033,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 168815.4,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 11127,
            "unit": "ms",
            "range": "344"
          },
          {
            "name": "Cached Build - source code change",
            "value": 41280.4,
            "unit": "ms",
            "range": "10360"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 41530.4,
            "unit": "ms",
            "range": "13399"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "kokoaj",
            "username": "kokiebisu",
            "email": "43525282+kokiebisu@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "ca5a2284d60134096ffcccbd4fd4a9655c8911f4",
          "message": "document fixes for the cache section (#782)",
          "timestamp": "2022-02-26T21:22:54Z",
          "url": "https://github.com/vercel/turborepo/commit/ca5a2284d60134096ffcccbd4fd4a9655c8911f4"
        },
        "date": 1646009001547,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 210499,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 11545.6,
            "unit": "ms",
            "range": "384"
          },
          {
            "name": "Cached Build - source code change",
            "value": 50600.4,
            "unit": "ms",
            "range": "11367"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 51098,
            "unit": "ms",
            "range": "10802"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Greg Soltis",
            "username": "gsoltis",
            "email": "greg.soltis@vercel.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "43b057ce2a75fb16e825263d997ed5fac35ba593",
          "message": "Running build with --cwd works (#783)\n\nCo-authored-by: Jared Palmer <jared@jaredpalmer.com>",
          "timestamp": "2022-02-28T22:53:07Z",
          "url": "https://github.com/vercel/turborepo/commit/43b057ce2a75fb16e825263d997ed5fac35ba593"
        },
        "date": 1646094725206,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 125748,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 10689.2,
            "unit": "ms",
            "range": "449"
          },
          {
            "name": "Cached Build - source code change",
            "value": 32015.6,
            "unit": "ms",
            "range": "7976"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 32862.6,
            "unit": "ms",
            "range": "7878"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Greg Soltis",
            "username": "gsoltis",
            "email": "greg.soltis@vercel.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "c9f7264d494b8cfc1c9f1f2c0762e2a2175b725c",
          "message": "Continue evolving `Context` towards a data structure that can be sent over the wire (#788)\n\n * `Args` doesn't need to hang off of `Context`\n * `TraceFilePath` isn't used, tracing is done via `RunState` instead.\n * global hash calculations and root `package.json` calculations can use local variables\n * `TaskGraph` is unused.",
          "timestamp": "2022-03-01T18:25:06Z",
          "url": "https://github.com/vercel/turborepo/commit/c9f7264d494b8cfc1c9f1f2c0762e2a2175b725c"
        },
        "date": 1646181224323,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 150454.8,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 11163.2,
            "unit": "ms",
            "range": "313"
          },
          {
            "name": "Cached Build - source code change",
            "value": 39072.8,
            "unit": "ms",
            "range": "8326"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 36665.4,
            "unit": "ms",
            "range": "8079"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "dependabot[bot]",
            "username": "dependabot[bot]",
            "email": "49699333+dependabot[bot]@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "2419e2fc98a04b698be98e54aae9536498fa15ec",
          "message": "Bump lint-staged from 12.5.0 to 13.0.0 (#1318)\n\nBumps [lint-staged](https://github.com/okonet/lint-staged) from 12.5.0 to 13.0.0.\r\n- [Release notes](https://github.com/okonet/lint-staged/releases)\r\n- [Commits](https://github.com/okonet/lint-staged/compare/v12.5.0...v13.0.0)\r\n\r\n---\r\nupdated-dependencies:\r\n- dependency-name: lint-staged\r\n  dependency-type: direct:development\r\n  update-type: version-update:semver-major\r\n...\r\n\r\nSigned-off-by: dependabot[bot] <support@github.com>\r\n\r\nCo-authored-by: dependabot[bot] <49699333+dependabot[bot]@users.noreply.github.com>\r\nCo-authored-by: Thomas Knickman <tom.knickman@vercel.com>",
          "timestamp": "2022-06-10T18:25:19Z",
          "url": "https://github.com/vercel/turborepo/commit/2419e2fc98a04b698be98e54aae9536498fa15ec"
        },
        "date": 1654907622328,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 148744.4,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 9930.6,
            "unit": "ms",
            "range": "1747"
          },
          {
            "name": "Cached Build - source code change",
            "value": 9609,
            "unit": "ms",
            "range": "138"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 36486,
            "unit": "ms",
            "range": "10537"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "dependabot[bot]",
            "username": "dependabot[bot]",
            "email": "49699333+dependabot[bot]@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "2419e2fc98a04b698be98e54aae9536498fa15ec",
          "message": "Bump lint-staged from 12.5.0 to 13.0.0 (#1318)\n\nBumps [lint-staged](https://github.com/okonet/lint-staged) from 12.5.0 to 13.0.0.\r\n- [Release notes](https://github.com/okonet/lint-staged/releases)\r\n- [Commits](https://github.com/okonet/lint-staged/compare/v12.5.0...v13.0.0)\r\n\r\n---\r\nupdated-dependencies:\r\n- dependency-name: lint-staged\r\n  dependency-type: direct:development\r\n  update-type: version-update:semver-major\r\n...\r\n\r\nSigned-off-by: dependabot[bot] <support@github.com>\r\n\r\nCo-authored-by: dependabot[bot] <49699333+dependabot[bot]@users.noreply.github.com>\r\nCo-authored-by: Thomas Knickman <tom.knickman@vercel.com>",
          "timestamp": "2022-06-10T18:25:19Z",
          "url": "https://github.com/vercel/turborepo/commit/2419e2fc98a04b698be98e54aae9536498fa15ec"
        },
        "date": 1654994266979,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 174602,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 10250.4,
            "unit": "ms",
            "range": "1058"
          },
          {
            "name": "Cached Build - source code change",
            "value": 10038.6,
            "unit": "ms",
            "range": "67"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 43083.8,
            "unit": "ms",
            "range": "11966"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "dependabot[bot]",
            "username": "dependabot[bot]",
            "email": "49699333+dependabot[bot]@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "2419e2fc98a04b698be98e54aae9536498fa15ec",
          "message": "Bump lint-staged from 12.5.0 to 13.0.0 (#1318)\n\nBumps [lint-staged](https://github.com/okonet/lint-staged) from 12.5.0 to 13.0.0.\r\n- [Release notes](https://github.com/okonet/lint-staged/releases)\r\n- [Commits](https://github.com/okonet/lint-staged/compare/v12.5.0...v13.0.0)\r\n\r\n---\r\nupdated-dependencies:\r\n- dependency-name: lint-staged\r\n  dependency-type: direct:development\r\n  update-type: version-update:semver-major\r\n...\r\n\r\nSigned-off-by: dependabot[bot] <support@github.com>\r\n\r\nCo-authored-by: dependabot[bot] <49699333+dependabot[bot]@users.noreply.github.com>\r\nCo-authored-by: Thomas Knickman <tom.knickman@vercel.com>",
          "timestamp": "2022-06-10T18:25:19Z",
          "url": "https://github.com/vercel/turborepo/commit/2419e2fc98a04b698be98e54aae9536498fa15ec"
        },
        "date": 1655080492899,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 142975.8,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 9603.8,
            "unit": "ms",
            "range": "1795"
          },
          {
            "name": "Cached Build - source code change",
            "value": 9257.6,
            "unit": "ms",
            "range": "184"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 34532.2,
            "unit": "ms",
            "range": "10603"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Greg Soltis",
            "username": "gsoltis",
            "email": "greg.soltis@vercel.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "1cba6a5fd19d4853b322cbad2cc3c925cc5d4753",
          "message": "Use Go 1.17 mode for lint (#1404)",
          "timestamp": "2022-06-13T22:18:06Z",
          "url": "https://github.com/vercel/turborepo/commit/1cba6a5fd19d4853b322cbad2cc3c925cc5d4753"
        },
        "date": 1655166952190,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 143849.4,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 9593.2,
            "unit": "ms",
            "range": "1568"
          },
          {
            "name": "Cached Build - source code change",
            "value": 9326.8,
            "unit": "ms",
            "range": "145"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 36941,
            "unit": "ms",
            "range": "8722"
          }
        ]
      }
    ]
  }
}