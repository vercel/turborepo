#!/usr/bin/env node
const { execSync } = require('child_process');
const fs = require('fs');
const path = require('path');

const evalsDir = '/Users/qua/vercel/next-evals-oss/evals';
const dirs = fs.readdirSync(evalsDir)
  .filter(d => !d.includes('node_modules') && !d.includes('package') && !d.endsWith('.json') && !d.endsWith('.yaml') && !d.startsWith('.'))
  .sort();

console.log(`Installing dependencies for ${dirs.length} eval folders...\n`);

let installed = 0;
let skipped = 0;

dirs.forEach((dir, index) => {
  const inputDir = path.join(evalsDir, dir, 'input');

  if (!fs.existsSync(inputDir)) {
    return;
  }

  const nodeModulesPath = path.join(inputDir, 'node_modules');

  // Skip if node_modules already exists
  if (fs.existsSync(nodeModulesPath)) {
    console.log(`⏭️  [${index + 1}/${dirs.length}] ${dir}: Dependencies already installed`);
    skipped++;
    return;
  }

  console.log(`📦 [${index + 1}/${dirs.length}] Installing ${dir}...`);

  try {
    execSync('pnpm install --reporter=silent', {
      cwd: inputDir,
      stdio: 'pipe',
      timeout: 180000,
    });

    console.log(`✅ [${index + 1}/${dirs.length}] ${dir}: Installed\n`);
    installed++;
  } catch (error) {
    console.log(`❌ [${index + 1}/${dirs.length}] ${dir}: Failed to install`);
    console.log(`Error: ${error.message.split('\n')[0]}\n`);
  }
});

console.log('\n' + '='.repeat(60));
console.log('INSTALLATION SUMMARY');
console.log('='.repeat(60));
console.log(`✅ Installed: ${installed}`);
console.log(`⏭️  Skipped: ${skipped}`);
