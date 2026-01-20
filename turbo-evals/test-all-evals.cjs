#!/usr/bin/env node
const { execSync } = require('child_process');
const fs = require('fs');
const path = require('path');

const evalsDir = '/Users/qua/vercel/next-evals-oss/evals';
const dirs = fs.readdirSync(evalsDir)
  .filter(d => !d.includes('node_modules') && !d.includes('package') && !d.endsWith('.json') && !d.endsWith('.yaml'))
  .sort();

const results = {
  passed: [],
  failed: [],
};

console.log(`Testing ${dirs.length} eval folders...\n`);

dirs.forEach((dir, index) => {
  const inputDir = path.join(evalsDir, dir, 'input');

  if (!fs.existsSync(inputDir)) {
    console.log(`⚠️  [${index + 1}/${dirs.length}] ${dir}: No input folder found`);
    return;
  }

  const packageJsonPath = path.join(inputDir, 'package.json');
  if (!fs.existsSync(packageJsonPath)) {
    console.log(`⚠️  [${index + 1}/${dirs.length}] ${dir}: No package.json found`);
    return;
  }

  console.log(`🔨 [${index + 1}/${dirs.length}] Testing ${dir}...`);

  try {
    // Run build-only
    execSync('pnpm build-only', {
      cwd: inputDir,
      stdio: 'pipe',
      timeout: 120000,
    });

    // Run lint
    execSync('pnpm lint', {
      cwd: inputDir,
      stdio: 'pipe',
      timeout: 60000,
    });

    console.log(`✅ [${index + 1}/${dirs.length}] ${dir}: PASSED\n`);
    results.passed.push(dir);
  } catch (error) {
    console.log(`❌ [${index + 1}/${dirs.length}] ${dir}: FAILED`);
    console.log(`Error: ${error.message.split('\n')[0]}\n`);
    results.failed.push({ dir, error: error.message });
  }
});

console.log('\n' + '='.repeat(60));
console.log('TEST SUMMARY');
console.log('='.repeat(60));
console.log(`✅ Passed: ${results.passed.length}/${dirs.length}`);
console.log(`❌ Failed: ${results.failed.length}/${dirs.length}`);

if (results.failed.length > 0) {
  console.log('\nFailed evals:');
  results.failed.forEach(({ dir }) => {
    console.log(`  - ${dir}`);
  });
  process.exit(1);
} else {
  console.log('\n🎉 All evals passed!');
  process.exit(0);
}
