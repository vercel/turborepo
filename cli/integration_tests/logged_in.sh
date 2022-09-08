#!/bin/sh

read -r -d '' CONFIG <<- EOF
{
  "token": "normal-user-token"
}
EOF

USER_CONFIG_HOME=$(mktemp -d -t turbo-XXXXXXXXXX)
# duplicate over to XDG var so that turbo picks it up
export XDG_CONFIG_HOME=$USER_CONFIG_HOME

mkdir -p $USER_CONFIG_HOME/turborepo
echo $CONFIG > $USER_CONFIG_HOME/turborepo/config.json
