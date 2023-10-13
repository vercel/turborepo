map({
    platform: .args.platform,
    name: "time-to-first-task",
    startTimeUnixMs: (if .args.start_time then .args.start_time | tonumber else null end),
    durationMs: (if .args.message == "running visitor" then .ts else null end),
    turboVersion: .args.turbo_version,
    scm: (if .args.scm_manual == "true" then "manual" elif .args.scm_manual == "false" then "git" else null end),
})
| reduce .[] as $item ({}; . *= ($item | del(.. | nulls)))
