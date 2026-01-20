#!/usr/bin/env node
const fs = require('fs');
const path = require('path');

const failedEvals = [
  '013-pathname-server',
  '021-avoid-fetch-in-effect',
  '022-prefer-server-actions',
  '023-avoid-getserversideprops',
  '026-no-serial-await',
  '030-app-router-migration-hard',
  '031-ai-sdk-migration-simple',
  '035-ai-sdk-call-tools',
  '036-ai-sdk-call-tools-multiple-steps',
];

const evalsDir = '/Users/qua/vercel/next-evals-oss/evals';

failedEvals.forEach(evalName => {
  const inputDir = path.join(evalsDir, evalName, 'input');

  if (!fs.existsSync(inputDir)) {
    console.log(`⚠️  ${evalName}: input directory not found`);
    return;
  }

  console.log(`🔧 Fixing ${evalName}...`);

  // Find all .ts and .tsx files
  const findFiles = (dir) => {
    let results = [];
    const list = fs.readdirSync(dir);

    list.forEach(file => {
      const filePath = path.join(dir, file);
      const stat = fs.statSync(filePath);

      if (stat && stat.isDirectory()) {
        if (file !== 'node_modules' && file !== '.next') {
          results = results.concat(findFiles(filePath));
        }
      } else if (file.endsWith('.ts') || file.endsWith('.tsx')) {
        results.push(filePath);
      }
    });

    return results;
  };

  const files = findFiles(inputDir);
  let fixedCount = 0;

  files.forEach(filePath => {
    let content = fs.readFileSync(filePath, 'utf8');
    const originalContent = content;

    // Replace common patterns with 'any' type
    content = content.replace(/: any>/g, ': unknown>');
    content = content.replace(/: any\)/g, ': unknown)');
    content = content.replace(/: any\[\]/g, ': unknown[]');
    content = content.replace(/: any;/g, ': unknown;');
    content = content.replace(/: any,/g, ': unknown,');
    content = content.replace(/<any>/g, '<unknown>');
    content = content.replace(/\(any\)/g, '(unknown)');
    content = content.replace(/Promise<any>/g, 'Promise<unknown>');
    content = content.replace(/Array<any>/g, 'Array<unknown>');

    // Replace 'as any' with 'as unknown'
    content = content.replace(/as any/g, 'as unknown');

    if (content !== originalContent) {
      fs.writeFileSync(filePath, content);
      fixedCount++;
      console.log(`  ✅ Fixed: ${path.relative(inputDir, filePath)}`);
    }
  });

  console.log(`  Total fixed in ${evalName}: ${fixedCount} files\n`);
});

console.log('Done! All any types have been replaced with unknown.');
