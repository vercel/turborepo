#!/usr/bin/env node
const fs = require('fs');
const path = require('path');

const evalsDir = '/Users/qua/vercel/next-evals-oss/evals';
const dirs = fs.readdirSync(evalsDir).filter(d => !d.includes('node_modules') && !d.includes('package'));

let fixed = 0;

dirs.forEach(dir => {
  const configPath = path.join(evalsDir, dir, 'input', 'next.config.ts');

  if (fs.existsSync(configPath)) {
    let content = fs.readFileSync(configPath, 'utf8');

    if (content.includes('mcpServer')) {
      // Replace the config with the mcpServer with an empty config
      content = content.replace(
        /const nextConfig: NextConfig = \{[\s\S]*?experimental: \{[\s\S]*?mcpServer: true,[\s\S]*?\},[\s\S]*?\};/,
        'const nextConfig: NextConfig = {};'
      );

      fs.writeFileSync(configPath, content);
      console.log(`Fixed: ${dir}`);
      fixed++;
    }
  }
});

console.log(`\nTotal fixed: ${fixed}`);
