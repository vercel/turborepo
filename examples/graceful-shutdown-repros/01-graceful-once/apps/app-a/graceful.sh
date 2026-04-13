set -u
trap 'printf "graceful cleanup start\n"; sleep 4.5; : > cleanup.done; printf "graceful cleanup done\n"; exit 0' INT
printf "graceful ready\n"
: >ready
while true; do sleep 0.2 || true; done
