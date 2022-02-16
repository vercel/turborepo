window.BENCHMARK_DATA = {
  "lastUpdate": 1645032112616,
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
      }
    ]
  }
}