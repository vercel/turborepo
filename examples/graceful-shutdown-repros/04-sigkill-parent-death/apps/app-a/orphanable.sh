set -u
sh -c 'trap "" TERM INT; while true; do sleep 0.2 || true; done' &
child=$!
printf '%s\n' "$child" > child.pid
printf "orphanable ready child=%s\n" "$child"
: > ready
while true; do sleep 0.2 || true; done
