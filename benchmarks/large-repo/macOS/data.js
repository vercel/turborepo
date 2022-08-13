window.BENCHMARK_DATA = {
  "lastUpdate": 1660350140676,
  "repoUrl": "https://github.com/vercel/turborepo",
  "entries": {
    "macOS Benchmark": [
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
        "date": 1644966290238,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 252203.4,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 12626.2,
            "unit": "ms",
            "range": "1456"
          },
          {
            "name": "Cached Build - source code change",
            "value": 50767.2,
            "unit": "ms",
            "range": "13043"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 51574.8,
            "unit": "ms",
            "range": "13481"
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
        "date": 1645029298163,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 247829.8,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 14253.8,
            "unit": "ms",
            "range": "5630"
          },
          {
            "name": "Cached Build - source code change",
            "value": 56405.2,
            "unit": "ms",
            "range": "29606"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 52535.2,
            "unit": "ms",
            "range": "8607"
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
        "date": 1645032111043,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 232900.2,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 11743.4,
            "unit": "ms",
            "range": "1409"
          },
          {
            "name": "Cached Build - source code change",
            "value": 52258,
            "unit": "ms",
            "range": "11593"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 53132.2,
            "unit": "ms",
            "range": "8870"
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
        "date": 1645032268007,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 266621,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 13563.4,
            "unit": "ms",
            "range": "2685"
          },
          {
            "name": "Cached Build - source code change",
            "value": 53755.8,
            "unit": "ms",
            "range": "15501"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 51623.4,
            "unit": "ms",
            "range": "11751"
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
        "date": 1645145153332,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 244807.4,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 15653.8,
            "unit": "ms",
            "range": "3289"
          },
          {
            "name": "Cached Build - source code change",
            "value": 75407.4,
            "unit": "ms",
            "range": "38295"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 58223.6,
            "unit": "ms",
            "range": "14269"
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
        "date": 1645231343391,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 200680.8,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 14898.4,
            "unit": "ms",
            "range": "5884"
          },
          {
            "name": "Cached Build - source code change",
            "value": 49965.2,
            "unit": "ms",
            "range": "7072"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 47930.8,
            "unit": "ms",
            "range": "10826"
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
        "date": 1645317800616,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 217077.2,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 12062.2,
            "unit": "ms",
            "range": "1444"
          },
          {
            "name": "Cached Build - source code change",
            "value": 48631.6,
            "unit": "ms",
            "range": "7015"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 49318.4,
            "unit": "ms",
            "range": "9092"
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
        "date": 1645404516128,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 241146,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 12208.2,
            "unit": "ms",
            "range": "1237"
          },
          {
            "name": "Cached Build - source code change",
            "value": 57012.4,
            "unit": "ms",
            "range": "19042"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 57347.2,
            "unit": "ms",
            "range": "8593"
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
        "date": 1645490831829,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 250359.8,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 13187,
            "unit": "ms",
            "range": "1663"
          },
          {
            "name": "Cached Build - source code change",
            "value": 69802.8,
            "unit": "ms",
            "range": "76522"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 51474.6,
            "unit": "ms",
            "range": "11326"
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
        "date": 1645577128201,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 250188.6,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 11714.2,
            "unit": "ms",
            "range": "2013"
          },
          {
            "name": "Cached Build - source code change",
            "value": 49758.6,
            "unit": "ms",
            "range": "9363"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 50933.2,
            "unit": "ms",
            "range": "9173"
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
        "date": 1645663330858,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 225893.2,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 12570.8,
            "unit": "ms",
            "range": "1624"
          },
          {
            "name": "Cached Build - source code change",
            "value": 49127.4,
            "unit": "ms",
            "range": "7730"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 49328.8,
            "unit": "ms",
            "range": "10612"
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
        "date": 1645749813257,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 228313.8,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 16128.4,
            "unit": "ms",
            "range": "14023"
          },
          {
            "name": "Cached Build - source code change",
            "value": 50045.4,
            "unit": "ms",
            "range": "10430"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 47745,
            "unit": "ms",
            "range": "11009"
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
        "date": 1645836202567,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 242155,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 12537.8,
            "unit": "ms",
            "range": "1880"
          },
          {
            "name": "Cached Build - source code change",
            "value": 49946.8,
            "unit": "ms",
            "range": "10366"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 49177,
            "unit": "ms",
            "range": "8834"
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
        "date": 1645923037422,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 295214.4,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 13312.8,
            "unit": "ms",
            "range": "1574"
          },
          {
            "name": "Cached Build - source code change",
            "value": 53621,
            "unit": "ms",
            "range": "17810"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 50639.4,
            "unit": "ms",
            "range": "8478"
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
        "date": 1646009005796,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 210312.8,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 12205.6,
            "unit": "ms",
            "range": "1189"
          },
          {
            "name": "Cached Build - source code change",
            "value": 49702.4,
            "unit": "ms",
            "range": "8094"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 50359.2,
            "unit": "ms",
            "range": "9995"
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
        "date": 1646095767617,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 250343.4,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 14141.6,
            "unit": "ms",
            "range": "1702"
          },
          {
            "name": "Cached Build - source code change",
            "value": 54115,
            "unit": "ms",
            "range": "15529"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 49655.2,
            "unit": "ms",
            "range": "9425"
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
        "date": 1646181705536,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 206943.2,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 11878,
            "unit": "ms",
            "range": "940"
          },
          {
            "name": "Cached Build - source code change",
            "value": 50379.4,
            "unit": "ms",
            "range": "7908"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 55084,
            "unit": "ms",
            "range": "9657"
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
        "date": 1654908482407,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 247468,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 8814.8,
            "unit": "ms",
            "range": "945"
          },
          {
            "name": "Cached Build - source code change",
            "value": 7997,
            "unit": "ms",
            "range": "806"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 53251.8,
            "unit": "ms",
            "range": "16335"
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
        "date": 1654994420635,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 201101.8,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 6652.6,
            "unit": "ms",
            "range": "205"
          },
          {
            "name": "Cached Build - source code change",
            "value": 6721.2,
            "unit": "ms",
            "range": "154"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 43459.2,
            "unit": "ms",
            "range": "9150"
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
        "date": 1655080973679,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 215842.8,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 7958.8,
            "unit": "ms",
            "range": "466"
          },
          {
            "name": "Cached Build - source code change",
            "value": 8012.4,
            "unit": "ms",
            "range": "213"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 48223.8,
            "unit": "ms",
            "range": "10687"
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
        "date": 1655167478070,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 232663.8,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 7949,
            "unit": "ms",
            "range": "888"
          },
          {
            "name": "Cached Build - source code change",
            "value": 7950,
            "unit": "ms",
            "range": "361"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 45361,
            "unit": "ms",
            "range": "8665"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Nathan Hammond",
            "username": "nathanhammond",
            "email": "nathan.hammond@vercel.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "0a6d8f1ab17db486520999bd9957c59def2ada2f",
          "message": "More CI Cleanup (#1408)\n\nThis makes our repo configuration more system-agnostic by pushing the platform-switching behavior farther to the roots.",
          "timestamp": "2022-06-14T12:46:37Z",
          "url": "https://github.com/vercel/turborepo/commit/0a6d8f1ab17db486520999bd9957c59def2ada2f"
        },
        "date": 1655253769941,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 216197.6,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 7333.6,
            "unit": "ms",
            "range": "1002"
          },
          {
            "name": "Cached Build - source code change",
            "value": 7267.8,
            "unit": "ms",
            "range": "271"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 49804.2,
            "unit": "ms",
            "range": "12973"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Nathan Hammond",
            "username": "nathanhammond",
            "email": "nathan.hammond@vercel.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "0a6d8f1ab17db486520999bd9957c59def2ada2f",
          "message": "More CI Cleanup (#1408)\n\nThis makes our repo configuration more system-agnostic by pushing the platform-switching behavior farther to the roots.",
          "timestamp": "2022-06-14T12:46:37Z",
          "url": "https://github.com/vercel/turborepo/commit/0a6d8f1ab17db486520999bd9957c59def2ada2f"
        },
        "date": 1655340108948,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 232013.4,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 9904.6,
            "unit": "ms",
            "range": "9204"
          },
          {
            "name": "Cached Build - source code change",
            "value": 8460.8,
            "unit": "ms",
            "range": "2799"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 48295,
            "unit": "ms",
            "range": "15342"
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
          "id": "501b6a62e82d8825588a75ae5a07a4b011461390",
          "message": "Add root boundary to untarring (#1409)\n\n* Add a test for untarring in the http cache\r\n\r\n* Disallow untarring files that cross the repo root boundary\r\n\r\n* Restructure resp.Body closing\r\n\r\n* Comments about tar headers being posix-style, and also our cache usage",
          "timestamp": "2022-06-16T18:16:41Z",
          "url": "https://github.com/vercel/turborepo/commit/501b6a62e82d8825588a75ae5a07a4b011461390"
        },
        "date": 1655426478859,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 219146,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 8269.6,
            "unit": "ms",
            "range": "2083"
          },
          {
            "name": "Cached Build - source code change",
            "value": 7939,
            "unit": "ms",
            "range": "1924"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 48385.2,
            "unit": "ms",
            "range": "10955"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Jeff Astor",
            "username": "Jastor11",
            "email": "jeff@astor.io"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "6e12d11e3aca1f063b87bbcba5aff654fa9a7b23",
          "message": "Friendly spellcheck in README.md (#1416)",
          "timestamp": "2022-06-17T16:08:29Z",
          "url": "https://github.com/vercel/turborepo/commit/6e12d11e3aca1f063b87bbcba5aff654fa9a7b23"
        },
        "date": 1655513044818,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 212889,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 8277.4,
            "unit": "ms",
            "range": "1334"
          },
          {
            "name": "Cached Build - source code change",
            "value": 7610.8,
            "unit": "ms",
            "range": "1296"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 48680.8,
            "unit": "ms",
            "range": "11024"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Jeff Astor",
            "username": "Jastor11",
            "email": "jeff@astor.io"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "6e12d11e3aca1f063b87bbcba5aff654fa9a7b23",
          "message": "Friendly spellcheck in README.md (#1416)",
          "timestamp": "2022-06-17T16:08:29Z",
          "url": "https://github.com/vercel/turborepo/commit/6e12d11e3aca1f063b87bbcba5aff654fa9a7b23"
        },
        "date": 1655599585558,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 260558,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 11167.8,
            "unit": "ms",
            "range": "5878"
          },
          {
            "name": "Cached Build - source code change",
            "value": 8718.4,
            "unit": "ms",
            "range": "850"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 61140.8,
            "unit": "ms",
            "range": "24579"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Jeff Astor",
            "username": "Jastor11",
            "email": "jeff@astor.io"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "6e12d11e3aca1f063b87bbcba5aff654fa9a7b23",
          "message": "Friendly spellcheck in README.md (#1416)",
          "timestamp": "2022-06-17T16:08:29Z",
          "url": "https://github.com/vercel/turborepo/commit/6e12d11e3aca1f063b87bbcba5aff654fa9a7b23"
        },
        "date": 1655685802006,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 217614.4,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 10639.2,
            "unit": "ms",
            "range": "7276"
          },
          {
            "name": "Cached Build - source code change",
            "value": 8856.8,
            "unit": "ms",
            "range": "1954"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 64111.8,
            "unit": "ms",
            "range": "12417"
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
          "id": "0ed93d29a15fb39f81e398b1cff9c4454666deab",
          "message": "Bump @react-aria/radio from 3.1.8 to 3.2.1 in /docs (#1421)\n\nBumps [@react-aria/radio](https://github.com/adobe/react-spectrum) from 3.1.8 to 3.2.1.\n<details>\n<summary>Commits</summary>\n<ul>\n<li><a href=\"https://github.com/adobe/react-spectrum/commit/1e9f5ad01acf1cad1099cdf2c96c604807d4f0cf\"><code>1e9f5ad</code></a> Publish</li>\n<li><a href=\"https://github.com/adobe/react-spectrum/commit/d5377632253f3d11122973ecaf6bdafb288ce930\"><code>d537763</code></a> Add DatePicker and Calendar to monopackages (<a href=\"https://github-redirect.dependabot.com/adobe/react-spectrum/issues/3234\">#3234</a>)</li>\n<li><a href=\"https://github.com/adobe/react-spectrum/commit/73ade29862f171cd6c37337ae76a880d6f598d32\"><code>73ade29</code></a> Fixing stuck FireFox ListView root drop indicator  (<a href=\"https://github-redirect.dependabot.com/adobe/react-spectrum/issues/3224\">#3224</a>)</li>\n<li><a href=\"https://github.com/adobe/react-spectrum/commit/4f3c81cd3bc14ea388564aa9d4572e963d010680\"><code>4f3c81c</code></a> Update TableView docs for checkbox/highlight + onAction behavior update (<a href=\"https://github-redirect.dependabot.com/adobe/react-spectrum/issues/3205\">#3205</a>)</li>\n<li><a href=\"https://github.com/adobe/react-spectrum/commit/922dbe8560618f9beecf5f183d39576536f93a29\"><code>922dbe8</code></a> CSF 3.0 Label and HelpText (<a href=\"https://github-redirect.dependabot.com/adobe/react-spectrum/issues/3152\">#3152</a>)</li>\n<li><a href=\"https://github.com/adobe/react-spectrum/commit/e5848f6deb058ea84f7ef1389ba53b16b1ed9a74\"><code>e5848f6</code></a> Work around Safari bug with ethiopic calendar (<a href=\"https://github-redirect.dependabot.com/adobe/react-spectrum/issues/3223\">#3223</a>)</li>\n<li><a href=\"https://github.com/adobe/react-spectrum/commit/df1783a7f62ea20183aaf14ce549e6c1f80046e5\"><code>df1783a</code></a> Fix FF date segment typing (<a href=\"https://github-redirect.dependabot.com/adobe/react-spectrum/issues/3222\">#3222</a>)</li>\n<li><a href=\"https://github.com/adobe/react-spectrum/commit/bcbe803b3591cb49a7cf45672e03bf122bc7cf5a\"><code>bcbe803</code></a> Fix entering dates with keyboard using VoiceOver on iOS (<a href=\"https://github-redirect.dependabot.com/adobe/react-spectrum/issues/3216\">#3216</a>)</li>\n<li><a href=\"https://github.com/adobe/react-spectrum/commit/b91e0f4841bda0912f18805b1bf5ba37a5732096\"><code>b91e0f4</code></a> Fix bugs with eras in DatePicker (<a href=\"https://github-redirect.dependabot.com/adobe/react-spectrum/issues/3215\">#3215</a>)</li>\n<li><a href=\"https://github.com/adobe/react-spectrum/commit/ce12e09a6b408b023717deab9e5a5602bb2c12fd\"><code>ce12e09</code></a> Shift focus when era is removed while focused in DatePicker (<a href=\"https://github-redirect.dependabot.com/adobe/react-spectrum/issues/3213\">#3213</a>)</li>\n<li>Additional commits viewable in <a href=\"https://github.com/adobe/react-spectrum/compare/@react-aria/radio@3.1.8...@react-aria/radio@3.2.1\">compare view</a></li>\n</ul>\n</details>\n<br />\n\n\n[![Dependabot compatibility score](https://dependabot-badges.githubapp.com/badges/compatibility_score?dependency-name=@react-aria/radio&package-manager=npm_and_yarn&previous-version=3.1.8&new-version=3.2.1)](https://docs.github.com/en/github/managing-security-vulnerabilities/about-dependabot-security-updates#about-compatibility-scores)\n\nDependabot will resolve any conflicts with this PR as long as you don't alter it yourself. You can also trigger a rebase manually by commenting `@dependabot rebase`.\n\n[//]: # (dependabot-automerge-start)\n[//]: # (dependabot-automerge-end)\n\n---\n\n<details>\n<summary>Dependabot commands and options</summary>\n<br />\n\nYou can trigger Dependabot actions by commenting on this PR:\n- `@dependabot rebase` will rebase this PR\n- `@dependabot recreate` will recreate this PR, overwriting any edits that have been made to it\n- `@dependabot merge` will merge this PR after your CI passes on it\n- `@dependabot squash and merge` will squash and merge this PR after your CI passes on it\n- `@dependabot cancel merge` will cancel a previously requested merge and block automerging\n- `@dependabot reopen` will reopen this PR if it is closed\n- `@dependabot close` will close this PR and stop Dependabot recreating it. You can achieve the same result by closing it manually\n- `@dependabot ignore this major version` will close this PR and stop Dependabot creating any more for this major version (unless you reopen the PR or upgrade to it yourself)\n- `@dependabot ignore this minor version` will close this PR and stop Dependabot creating any more for this minor version (unless you reopen the PR or upgrade to it yourself)\n- `@dependabot ignore this dependency` will close this PR and stop Dependabot creating any more for this dependency (unless you reopen the PR or upgrade to it yourself)\n\n\n</details>\n\nCo-authored-by: Thomas Knickman <2933988+tknickman@users.noreply.github.com>",
          "timestamp": "2022-06-20T19:16:24Z",
          "url": "https://github.com/vercel/turborepo/commit/0ed93d29a15fb39f81e398b1cff9c4454666deab"
        },
        "date": 1655772021395,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 204582.2,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 7828.2,
            "unit": "ms",
            "range": "371"
          },
          {
            "name": "Cached Build - source code change",
            "value": 7852.2,
            "unit": "ms",
            "range": "485"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 44528,
            "unit": "ms",
            "range": "10381"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Thomas Knickman",
            "username": "tknickman",
            "email": "tom.knickman@vercel.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "793dca02682a7618a680beaf5cb38e7df357b43f",
          "message": "feat(cli): update the graph arg behavior (#1353)\n\nUpdates the behavior of the `--graph` CLI flag and fixes a few bugs. \n\n> This command will generate an svg, png, jpg, pdf, json, html, or [other supported output formats](https://graphviz.org/doc/info/output.html) of the current task graph.\nThe output file format defaults to jpg, but can be controlled by specifying the filename's extension.\n\n> If Graphviz is not installed, or no filename is provided, this command prints the dot graph to `stdout`\n\nThis PR also:\n1. Updates docs to reflect the current state of the `--graph` CLI flag\n1. Refactors the graph visualization code out of `run.go`\n1. Cleans up the file name of colors_cache (follow up from https://github.com/vercel/turborepo/pull/1346)\n\n\nFixes https://github.com/vercel/turborepo/issues/1286",
          "timestamp": "2022-06-21T18:32:34Z",
          "url": "https://github.com/vercel/turborepo/commit/793dca02682a7618a680beaf5cb38e7df357b43f"
        },
        "date": 1655858856850,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 232885.2,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 7925.2,
            "unit": "ms",
            "range": "751"
          },
          {
            "name": "Cached Build - source code change",
            "value": 7969.4,
            "unit": "ms",
            "range": "247"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 47221.8,
            "unit": "ms",
            "range": "10539"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Ender Bonnet",
            "username": "enBonnet",
            "email": "13243693+enBonnet@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "1943e2dadedf85c9cae4bb4eef21e846582e44d8",
          "message": "Remove duplicate \"have\" (#1430)",
          "timestamp": "2022-06-22T19:07:08Z",
          "url": "https://github.com/vercel/turborepo/commit/1943e2dadedf85c9cae4bb4eef21e846582e44d8"
        },
        "date": 1655944736873,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 206729,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 6540.4,
            "unit": "ms",
            "range": "67"
          },
          {
            "name": "Cached Build - source code change",
            "value": 6658.4,
            "unit": "ms",
            "range": "190"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 44878.6,
            "unit": "ms",
            "range": "9194"
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
            "name": "Jared Palmer",
            "username": "jaredpalmer",
            "email": "jared@jaredpalmer.com"
          },
          "id": "fbea0d6ecd49a8c6ff9bdb4fea0425345fc43553",
          "message": "publish 1.3.1 to registry",
          "timestamp": "2022-06-23T22:51:59Z",
          "url": "https://github.com/vercel/turborepo/commit/fbea0d6ecd49a8c6ff9bdb4fea0425345fc43553"
        },
        "date": 1656031256210,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 230873.2,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 7902.2,
            "unit": "ms",
            "range": "158"
          },
          {
            "name": "Cached Build - source code change",
            "value": 8126.6,
            "unit": "ms",
            "range": "105"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 44855.6,
            "unit": "ms",
            "range": "11799"
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
            "name": "Jared Palmer",
            "username": "jaredpalmer",
            "email": "jared@jaredpalmer.com"
          },
          "id": "9b04fc04b73100e57c316563e73bec7b172007b7",
          "message": "Improve seo of config page",
          "timestamp": "2022-06-24T12:44:27Z",
          "url": "https://github.com/vercel/turborepo/commit/9b04fc04b73100e57c316563e73bec7b172007b7"
        },
        "date": 1656117797645,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 214157.8,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 8395.6,
            "unit": "ms",
            "range": "895"
          },
          {
            "name": "Cached Build - source code change",
            "value": 8874.4,
            "unit": "ms",
            "range": "390"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 71063.8,
            "unit": "ms",
            "range": "34690"
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
            "name": "Jared Palmer",
            "username": "jaredpalmer",
            "email": "jared@jaredpalmer.com"
          },
          "id": "9b04fc04b73100e57c316563e73bec7b172007b7",
          "message": "Improve seo of config page",
          "timestamp": "2022-06-24T12:44:27Z",
          "url": "https://github.com/vercel/turborepo/commit/9b04fc04b73100e57c316563e73bec7b172007b7"
        },
        "date": 1656204510941,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 241072.4,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 9027.6,
            "unit": "ms",
            "range": "2124"
          },
          {
            "name": "Cached Build - source code change",
            "value": 8747.4,
            "unit": "ms",
            "range": "176"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 68218.2,
            "unit": "ms",
            "range": "20324"
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
            "name": "Jared Palmer",
            "username": "jaredpalmer",
            "email": "jared@jaredpalmer.com"
          },
          "id": "9b04fc04b73100e57c316563e73bec7b172007b7",
          "message": "Improve seo of config page",
          "timestamp": "2022-06-24T12:44:27Z",
          "url": "https://github.com/vercel/turborepo/commit/9b04fc04b73100e57c316563e73bec7b172007b7"
        },
        "date": 1656290516945,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 213233.4,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 6868,
            "unit": "ms",
            "range": "96"
          },
          {
            "name": "Cached Build - source code change",
            "value": 7057.2,
            "unit": "ms",
            "range": "172"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 45107.4,
            "unit": "ms",
            "range": "9303"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Mehul Kar",
            "username": "mehulkar",
            "email": "mehul.kar@gmail.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "ca3b50a0e23857791255647aa763bc4db1d9818a",
          "message": "Remove references to baseBranch (#1681)",
          "timestamp": "2022-08-12T22:37:59Z",
          "url": "https://github.com/vercel/turborepo/commit/ca3b50a0e23857791255647aa763bc4db1d9818a"
        },
        "date": 1660350139536,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Clean Build",
            "value": 239762.2,
            "unit": "ms",
            "range": "0"
          },
          {
            "name": "Cached Build - no changes",
            "value": 9729.8,
            "unit": "ms",
            "range": "1676"
          },
          {
            "name": "Cached Build - source code change",
            "value": 9177.8,
            "unit": "ms",
            "range": "1101"
          },
          {
            "name": "Cached Build - dependency change",
            "value": 72013.2,
            "unit": "ms",
            "range": "37567"
          }
        ]
      }
    ]
  }
}