#!/usr/bin/env node
const { execSync } = require('child_process');
const fs = require('fs');
const path = require('path');

const evalsDir = '/Users/qua/vercel/next-evals-oss/evals';
const dirs = fs.readdirSync(evalsDir)
  .filter(d => !d.includes('node_modules') && !d.includes('package') && !d.endsWith('.json') && !d.endsWith('.yaml') && !d.startsWith('.'))
  .sort();

const results = {
  passed: [],
  failed: [],
  fixed: [],
};

console.log(`Fixing and testing ${dirs.length} eval folders...\n`);

dirs.forEach((dir, index) => {
  const inputDir = path.join(evalsDir, dir, 'input');

  if (!fs.existsSync(inputDir)) {
    return;
  }

  console.log(`\n🔍 [${index + 1}/${dirs.length}] Processing ${dir}...`);

  const nodeModulesPath = path.join(inputDir, 'node_modules');
  const nextBinary = path.join(nodeModulesPath, '.bin', 'next');

  // Check if next binary exists
  if (!fs.existsSync(nextBinary)) {
    console.log(`  📦 Reinstalling dependencies...`);

    try {
      execSync('rm -rf node_modules pnpm-lock.yaml && pnpm install --reporter=silent', {
        cwd: inputDir,
        stdio: 'pipe',
        timeout: 180000,
      });
      console.log(`  ✅ Dependencies installed`);
      results.fixed.push(dir);
    } catch (error) {
      console.log(`  ❌ Failed to install dependencies`);
      results.failed.push({ dir, error: 'Failed to install dependencies' });
      return;
    }
  }

  // Try to build
  try {
    execSync('pnpm build-only', {
      cwd: inputDir,
      stdio: 'pipe',
      timeout: 120000,
    });

    execSync('pnpm lint', {
      cwd: inputDir,
      stdio: 'pipe',
      timeout: 60000,
    });

    console.log(`  ✅ PASSED`);
    results.passed.push(dir);
  } catch (error) {
    console.log(`  ❌ FAILED`);
    const errorOutput = error.stderr?.toString() || error.stdout?.toString() || error.message;
    results.failed.push({ dir, error: errorOutput.slice(0, 500) });
  }
});

console.log('\n' + '='.repeat(60));
console.log('SUMMARY');
console.log('='.repeat(60));
console.log(`✅ Passed: ${results.passed.length}/${dirs.length}`);
console.log(`❌ Failed: ${results.failed.length}/${dirs.length}`);
console.log(`🔧 Fixed node_modules: ${results.fixed.length}`);

if (results.failed.length > 0) {
  console.log('\nFailed evals:');
  results.failed.forEach(({ dir, error }) => {
    console.log(`\n  ${dir}:`);
    if (error.includes('no-explicit-any')) {
      console.log(`    Issue: TypeScript 'any' type detected`);
    } else if (error.includes('Failed to compile')) {
      console.log(`    Issue: Compilation error`);
    } else {
      console.log(`    ${error.split('\n')[0]}`);
    }
  });
  process.exit(1);
}

console.log('\n🎉 All evals passed!');
process.exit(0);
