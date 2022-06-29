cat ../dist/artifacts.json | jq -r '.[] | select(.type == "Binary") | "cp dist/" + .name + " npm/" + .name' | sh
