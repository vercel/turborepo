// Build output depends on CI_DEPLOY_KEY -- but only when CI is set.
if (process.env.CI) {
  console.log('deploy target:', process.env.CI_DEPLOY_KEY);
} else {
  console.log('local build');
}
