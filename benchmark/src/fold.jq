map({
    platform: .args.platform,
    name: "time-to-first-task",
    startTimeUnixMicroseconds: (if .args.start_time then .args.start_time | tonumber else null end),
    durationMicroseconds: (if .args.message == "running visitor" then .ts else null end),
    turboVersion: .args.turbo_version,
    scm: "git",
})
| reduce .[] as $item ({}; . *= ($item | del(.. | nulls)))
