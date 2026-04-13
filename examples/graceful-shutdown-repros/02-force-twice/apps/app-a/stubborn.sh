set -u
trap '' INT
sh -c 'trap "" INT TERM; while true; do sleep 0.2 || true; done' &
child=$!
printf '%s\n' "$child" > child.pid
printf "stubborn ready child=%s\n" "$child"
: > ready
while true; do sleep 0.2 || true; done
