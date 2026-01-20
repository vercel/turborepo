#!/bin/bash
# Generate test files for agent evals based on their prompts

# agent-004-search-params
cat > evals/agent-004-search-params/input/app/page.test.tsx << 'EOF'
import { expect, test } from 'vitest';
import { readFileSync } from 'fs';
import { join } from 'path';

test('Page is an async server component', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  // Should be async function
  expect(pageContent).toMatch(/export\s+default\s+async\s+function|async\s+function.*Page/);

  // Should NOT have 'use client'
  expect(pageContent).not.toMatch(/['"]use client['"];?/);
});

test('Page reads searchParams', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  // Should accept searchParams prop
  expect(pageContent).toMatch(/searchParams/);

  // Should await searchParams (Next.js 15)
  expect(pageContent).toMatch(/await\s+searchParams/);
});

test('Page displays name from URL parameter', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  // Should read 'name' parameter
  expect(pageContent).toMatch(/\.name|searchParams\['name'\]|searchParams\["name"\]/);

  // Should display Hello message
  expect(pageContent).toMatch(/Hello/);
});

test('Page has fallback for missing name', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  // Should have fallback like "World" or similar
  expect(pageContent).toMatch(/World|\|\||??/);
});
EOF

# agent-005-react-use-api
cat > evals/agent-005-react-use-api/input/app/page.test.tsx << 'EOF'
import { expect, test } from 'vitest';
import { readFileSync, readdirSync } from 'fs';
import { join } from 'path';

test('Page has async data function', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  // Should have async function that returns data
  expect(pageContent).toMatch(/async\s+function.*{.*message/s);
});

test('Page has Client Component using use() hook', () => {
  const files = readdirSync(join(process.cwd(), 'app'));
  const hasClientComponent = files.some(f =>
    f.match(/client|component/i) && f.endsWith('.tsx')
  );

  if (hasClientComponent) {
    const clientFiles = files.filter(f =>
      f.match(/client|component/i) && f.endsWith('.tsx')
    );
    const clientContent = readFileSync(
      join(process.cwd(), 'app', clientFiles[0]),
      'utf-8'
    );

    // Should have 'use client'
    expect(clientContent).toMatch(/['"]use client['"];?/);

    // Should import and use the use() hook
    expect(clientContent).toMatch(/import.*use.*from.*react/);
    expect(clientContent).toMatch(/use\(/);
  } else {
    // Check in main page file
    const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');
    expect(pageContent).toMatch(/use\(/);
  }
});

test('Page uses Suspense', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  // Should use Suspense component
  expect(pageContent).toMatch(/<Suspense|Suspense>/);
  expect(pageContent).toMatch(/fallback/);
});
EOF

echo "Generated test files for agent evals 004-005"
echo "Run this script to generate more as needed"
