echo "::group::add"
pnpm add -g node-gyp
echo "::endgroup::"

echo "::group::list installed node development files"
pnpm exec node-gyp --verbose list
echo "::endgroup::"

echo "::group::install node development files"
pnpm exec node-gyp --verbose install $(node -v)
echo "::endgroup::"

echo "::group::list installed node development files"
pnpm exec node-gyp --verbose list
echo "::endgroup::"

# TODO resolve "node-gyp" cache path or set it for "node-gyp" explicitly rather than hardcoding the value

echo "::group::ls"
ls "C:\Users\runneradmin\AppData\Local\node-gyp\Cache\$($(node -v).TrimStart('v'))\include\node"
echo "::endgroup::"

echo "::group::list pnpm cache dir"
echo "$(pnpm list --global --parseable | select -First 1)"
echo "::endgroup::"

#echo "::group::set npm_config_node_gyp env var"
#echo "npm_config_node_gyp=$(pnpm list --global --parseable | select -First 1)/node_modules/node_gyp" >> $GITHUB_ENV
#echo "::endgroup::"