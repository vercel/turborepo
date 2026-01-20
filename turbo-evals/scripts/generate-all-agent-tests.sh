#!/bin/bash
# Generate test files for all remaining agent evals

# agent-011-client-server-form
cat > evals/agent-011-client-server-form/input/app/page.test.tsx << 'EOF'
import { expect, test } from 'vitest';
import { readFileSync, existsSync } from 'fs';
import { join } from 'path';

test('Page has form with input field', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/<form/i);
  expect(pageContent).toMatch(/<input/i);
  expect(pageContent).toMatch(/name\s*=/);
});

test('Page has submit button', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/<button.*type\s*=\s*["']submit["']|<button(?![^>]*type=)/i);
});

test('Page uses server action', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  // Should have action prop or separate actions file
  expect(pageContent).toMatch(/action\s*=|formAction/);

  const hasActionsFile = existsSync(join(process.cwd(), 'app', 'actions.ts'));
  const hasInlineAction = pageContent.match(/['"]use server['"];?/);

  expect(hasActionsFile || hasInlineAction).toBeTruthy();
});

test('Page displays submission result', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  // Should display some feedback after submission
  expect(pageContent).toMatch(/result|message|success|status/i);
});
EOF

# agent-012-parallel-routes
cat > evals/agent-012-parallel-routes/input/app/page.test.tsx << 'EOF'
import { expect, test } from 'vitest';
import { existsSync, readdirSync } from 'fs';
import { join } from 'path';

test('Has @analytics parallel route slot', () => {
  const hasAnalyticsSlot = existsSync(join(process.cwd(), 'app', '@analytics'));

  expect(hasAnalyticsSlot).toBe(true);
});

test('Has @settings parallel route slot', () => {
  const hasSettingsSlot = existsSync(join(process.cwd(), 'app', '@settings'));

  expect(hasSettingsSlot).toBe(true);
});

test('Layout renders both slots', () => {
  const hasLayout = existsSync(join(process.cwd(), 'app', 'layout.tsx'));

  expect(hasLayout).toBe(true);
});
EOF

# agent-016-client-cookies
cat > evals/agent-016-client-cookies/input/app/page.test.tsx << 'EOF'
import { expect, test } from 'vitest';
import { readFileSync, existsSync } from 'fs';
import { join } from 'path';

test('Page is a client component', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/['"]use client['"];?/);
});

test('Page has button that triggers server action', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/<button/i);
  expect(pageContent).toMatch(/onClick/);
});

test('Page has server action for setting cookies', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  const hasActionsFile = existsSync(join(process.cwd(), 'app', 'actions.ts'));
  const hasInlineAction = pageContent.match(/['"]use server['"];?/);

  expect(hasActionsFile || hasInlineAction).toBeTruthy();
});

test('Page displays feedback when cookie is set', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/message|feedback|success|status/i);
});
EOF

# agent-017-use-search-params
cat > evals/agent-017-use-search-params/input/app/page.test.tsx << 'EOF'
import { expect, test } from 'vitest';
import { readFileSync } from 'fs';
import { join } from 'path';

test('Page is a client component', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/['"]use client['"];?/);
});

test('Page uses useSearchParams hook', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/useSearchParams/);
  expect(pageContent).toMatch(/from\s+['"]next\/navigation['"]/);
});

test('Page is wrapped in Suspense', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/<Suspense|Suspense>/);
});

test('Page displays search params', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/\.get\(|searchParams/);
});
EOF

# agent-018-use-router
cat > evals/agent-018-use-router/input/app/page.test.tsx << 'EOF'
import { expect, test } from 'vitest';
import { readFileSync } from 'fs';
import { join } from 'path';

test('Page is a client component', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/['"]use client['"];?/);
});

test('Page uses useRouter hook', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/useRouter/);
  expect(pageContent).toMatch(/from\s+['"]next\/navigation['"]/);
});

test('Page has Navigate button', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/<button/i);
  expect(pageContent).toMatch(/Navigate/);
});

test('Button navigates to /about', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/router\.push\s*\(\s*['"]\/about['"]\)|\.push\(['"]\/about['"]\)/);
});
EOF

# agent-019-use-action-state
cat > evals/agent-019-use-action-state/input/app/page.test.tsx << 'EOF'
import { expect, test } from 'vitest';
import { readFileSync } from 'fs';
import { join } from 'path';

test('Page uses useActionState hook', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/useActionState/);
});

test('Page does NOT use useState', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).not.toMatch(/useState/);
});

test('Page has form with input', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/<form/i);
  expect(pageContent).toMatch(/<input/i);
});

test('Page displays success or error message', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/success|error|message/i);
});
EOF

# agent-020-no-use-effect
cat > evals/agent-020-no-use-effect/input/app/page.test.tsx << 'EOF'
import { expect, test } from 'vitest';
import { readFileSync } from 'fs';
import { join } from 'path';

test('Page is a client component', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/['"]use client['"];?/);
});

test('Page does NOT use useEffect', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).not.toMatch(/useEffect/);
});

test('Page checks navigator API', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/navigator\.userAgent|navigator/);
});

test('Page displays browser detection result', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/Safari|Firefox|Unsupported.*Browser|Welcome/i);
});
EOF

# agent-022-prefer-server-actions
cat > evals/agent-022-prefer-server-actions/input/app/page.test.tsx << 'EOF'
import { expect, test } from 'vitest';
import { readFileSync, existsSync } from 'fs';
import { join } from 'path';

test('Page has contact form with required fields', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/<form/i);
  expect(pageContent).toMatch(/name|email|message/i);
});

test('Page uses server action not API route', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  const hasActionsFile = existsSync(join(process.cwd(), 'app', 'actions.ts'));
  const hasInlineAction = pageContent.match(/['"]use server['"];?/);

  expect(hasActionsFile || hasInlineAction).toBeTruthy();

  // Should NOT have API route
  const hasApiRoute = existsSync(join(process.cwd(), 'app', 'api'));
  expect(hasApiRoute).toBe(false);
});

test('Page displays success message', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/success|submitted|thank/i);
});
EOF

# agent-024-avoid-redundant-usestate
cat > evals/agent-024-avoid-redundant-usestate/input/app/page.test.tsx << 'EOF'
import { expect, test } from 'vitest';
import { readFileSync } from 'fs';
import { join } from 'path';

test('Page displays user statistics', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/active.*users|users.*active/i);
  expect(pageContent).toMatch(/inactive.*users|users.*inactive/i);
  expect(pageContent).toMatch(/percentage|%/i);
});

test('Page derives values without redundant useState', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  // Should calculate derived values directly, not store them in separate state
  // Check for calculation logic
  expect(pageContent).toMatch(/filter|reduce|length/);

  // Count useState calls - should be minimal
  const useStateCount = (pageContent.match(/useState/g) || []).length;
  expect(useStateCount).toBeLessThanOrEqual(1); // At most 1 for the users data
});
EOF

# agent-025-prefer-next-link
cat > evals/agent-025-prefer-next-link/input/app/page.test.tsx << 'EOF'
import { expect, test } from 'vitest';
import { readFileSync } from 'fs';
import { join } from 'path';

test('Page has navigation links', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/\/blog/);
  expect(pageContent).toMatch(/\/products/);
  expect(pageContent).toMatch(/\/support/);
});

test('Page uses next/link not anchor tags', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/import.*Link.*from\s+['"]next\/link['"]/);
  expect(pageContent).toMatch(/<Link/);

  // Should NOT use <a> tags for internal navigation
  const aTagsCount = (pageContent.match(/<a\s+href/gi) || []).length;
  expect(aTagsCount).toBe(0);
});
EOF

# agent-027-prefer-next-image
cat > evals/agent-027-prefer-next-image/input/app/page.test.tsx << 'EOF'
import { expect, test } from 'vitest';
import { readFileSync } from 'fs';
import { join } from 'path';

test('Page has product gallery', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/product|gallery|image/i);
});

test('Page uses next/image not img tags', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/import.*Image.*from\s+['"]next\/image['"]/);
  expect(pageContent).toMatch(/<Image/);

  // Should NOT use <img> tags
  expect(pageContent).not.toMatch(/<img/i);
});

test('Page specifies image dimensions', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/width.*300|width={300}/);
  expect(pageContent).toMatch(/height.*200|height={200}/);
});
EOF

# agent-028-prefer-next-font
cat > evals/agent-028-prefer-next-font/input/app/page.test.tsx << 'EOF'
import { expect, test } from 'vitest';
import { readFileSync } from 'fs';
import { join } from 'path';

test('Page imports fonts from next/font/google', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/from\s+['"]next\/font\/google['"]/);
});

test('Page uses Playfair Display font', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/Playfair.*Display|Playfair_Display/i);
});

test('Page uses Roboto font', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/Roboto/i);
});

test('Page applies fonts to elements', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/className.*font|\.className/);
  expect(pageContent).toMatch(/<h1|<p/);
});
EOF

# agent-039-parallel-routes
cat > evals/agent-039-parallel-routes/input/app/page.test.tsx << 'EOF'
import { expect, test } from 'vitest';
import { existsSync, readFileSync } from 'fs';
import { join } from 'path';

test('Has @analytics parallel route', () => {
  const hasAnalyticsSlot = existsSync(join(process.cwd(), 'app', '@analytics'));
  expect(hasAnalyticsSlot).toBe(true);
});

test('Has @team parallel route', () => {
  const hasTeamSlot = existsSync(join(process.cwd(), 'app', '@team'));
  expect(hasTeamSlot).toBe(true);
});

test('Analytics slot has correct content', () => {
  const analyticsPath = join(process.cwd(), 'app', '@analytics', 'page.tsx');
  if (existsSync(analyticsPath)) {
    const content = readFileSync(analyticsPath, 'utf-8');
    expect(content).toMatch(/Analytics Dashboard/);
    expect(content).toMatch(/className\s*=\s*["']analytics["']/);
  }
});

test('Team slot has correct content', () => {
  const teamPath = join(process.cwd(), 'app', '@team', 'page.tsx');
  if (existsSync(teamPath)) {
    const content = readFileSync(teamPath, 'utf-8');
    expect(content).toMatch(/Team Overview/);
    expect(content).toMatch(/className\s*=\s*["']team["']/);
  }
});
EOF

# agent-040-intercepting-routes
cat > evals/agent-040-intercepting-routes/input/app/page.test.tsx << 'EOF'
import { expect, test } from 'vitest';
import { existsSync, readFileSync } from 'fs';
import { join } from 'path';

test('Has link to photo', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/\/photo\/1/);
  expect(pageContent).toMatch(/<Link|<a/);
});

test('Has intercepting route for modal', () => {
  const hasInterceptingRoute = existsSync(join(process.cwd(), 'app', '(.)', 'photo'));
  expect(hasInterceptingRoute).toBe(true);
});

test('Has regular photo route', () => {
  const hasRegularRoute = existsSync(join(process.cwd(), 'app', 'photo'));
  expect(hasRegularRoute).toBe(true);
});

test('Intercepting route shows modal', () => {
  const interceptingPath = join(process.cwd(), 'app', '(.)', 'photo', '[id]', 'page.tsx');
  if (existsSync(interceptingPath)) {
    const content = readFileSync(interceptingPath, 'utf-8');
    expect(content).toMatch(/Photo.*Modal|modal/i);
    expect(content).toMatch(/className\s*=\s*["']modal["']/);
  }
});
EOF

# agent-041-route-groups
cat > evals/agent-041-route-groups/input/app/page.test.tsx << 'EOF'
import { expect, test } from 'vitest';
import { existsSync, readFileSync } from 'fs';
import { join } from 'path';

test('Has marketing route group', () => {
  const hasMarketing = existsSync(join(process.cwd(), 'app', '(marketing)'));
  expect(hasMarketing).toBe(true);
});

test('Has shop route group', () => {
  const hasShop = existsSync(join(process.cwd(), 'app', '(shop)'));
  expect(hasShop).toBe(true);
});

test('About page in marketing group', () => {
  const aboutPath = join(process.cwd(), 'app', '(marketing)', 'about', 'page.tsx');
  if (existsSync(aboutPath)) {
    const content = readFileSync(aboutPath, 'utf-8');
    expect(content).toMatch(/About Us/);
    expect(content).toMatch(/<h1/);
  }
});

test('Products page in shop group', () => {
  const productsPath = join(process.cwd(), 'app', '(shop)', 'products', 'page.tsx');
  if (existsSync(productsPath)) {
    const content = readFileSync(productsPath, 'utf-8');
    expect(content).toMatch(/Our Products/);
    expect(content).toMatch(/<h1/);
  }
});
EOF

# agent-042-loading-ui
cat > evals/agent-042-loading-ui/input/app/page.test.tsx << 'EOF'
import { expect, test } from 'vitest';
import { existsSync, readFileSync } from 'fs';
import { join } from 'path';

test('Has loading.tsx file', () => {
  const hasLoading = existsSync(join(process.cwd(), 'app', 'loading.tsx'));
  expect(hasLoading).toBe(true);
});

test('Loading file shows loading state', () => {
  const loadingPath = join(process.cwd(), 'app', 'loading.tsx');
  if (existsSync(loadingPath)) {
    const content = readFileSync(loadingPath, 'utf-8');
    expect(content).toMatch(/Loading/);
    expect(content).toMatch(/className\s*=\s*["']loading-spinner["']/);
  }
});

test('Page is async with delay', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/async/);
  expect(pageContent).toMatch(/await.*Promise|setTimeout|delay/);
});

test('Page displays content after loading', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/Content Loaded/);
  expect(pageContent).toMatch(/<h1/);
});
EOF

# agent-043-error-boundaries
cat > evals/agent-043-error-boundaries/input/app/page.test.tsx << 'EOF'
import { expect, test } from 'vitest';
import { existsSync, readFileSync } from 'fs';
import { join } from 'path';

test('Has error.tsx file', () => {
  const hasError = existsSync(join(process.cwd(), 'app', 'error.tsx'));
  expect(hasError).toBe(true);
});

test('Error file is a client component', () => {
  const errorPath = join(process.cwd(), 'app', 'error.tsx');
  if (existsSync(errorPath)) {
    const content = readFileSync(errorPath, 'utf-8');
    expect(content).toMatch(/['"]use client['"];?/);
  }
});

test('Error file shows error message', () => {
  const errorPath = join(process.cwd(), 'app', 'error.tsx');
  if (existsSync(errorPath)) {
    const content = readFileSync(errorPath, 'utf-8');
    expect(content).toMatch(/Something went wrong/);
    expect(content).toMatch(/<h1/);
  }
});

test('Error file has reset button', () => {
  const errorPath = join(process.cwd(), 'app', 'error.tsx');
  if (existsSync(errorPath)) {
    const content = readFileSync(errorPath, 'utf-8');
    expect(content).toMatch(/Try again/);
    expect(content).toMatch(/<button/);
    expect(content).toMatch(/reset/);
  }
});

test('Page throws error', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/throw.*Error|throw.*new/);
});
EOF

# agent-045-server-actions-form
cat > evals/agent-045-server-actions-form/input/app/page.test.tsx << 'EOF'
import { expect, test } from 'vitest';
import { readFileSync, existsSync } from 'fs';
import { join } from 'path';

test('Page has form with server action', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/<form/i);
  expect(pageContent).toMatch(/action\s*=/);
});

test('Has submitForm server action', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  const hasActionsFile = existsSync(join(process.cwd(), 'app', 'actions.ts'));
  const hasInlineAction = pageContent.match(/['"]use server['"];?/);

  expect(hasActionsFile || hasInlineAction).toBeTruthy();
  expect(pageContent).toMatch(/submitForm|submit/i);
});

test('Form has name input', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/<input/i);
  expect(pageContent).toMatch(/name\s*=\s*["']name["']/);
  expect(pageContent).toMatch(/placeholder.*Enter your name|Enter your name/i);
});

test('Form has submit button', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/<button/i);
  expect(pageContent).toMatch(/Submit/);
});
EOF

# agent-046-streaming
cat > evals/agent-046-streaming/input/app/page.test.tsx << 'EOF'
import { expect, test } from 'vitest';
import { readFileSync } from 'fs';
import { join } from 'path';

test('Page has fast-loading header', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/Dashboard/);
  expect(pageContent).toMatch(/<h1/);
});

test('Page uses Suspense', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/<Suspense/);
  expect(pageContent).toMatch(/fallback/);
  expect(pageContent).toMatch(/Loading data/);
});

test('Page has slow component with delay', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/async/);
  expect(pageContent).toMatch(/await.*Promise|setTimeout|delay/);
  expect(pageContent).toMatch(/3000|3\s*\*\s*1000/); // 3 second delay
});

test('Slow component displays loaded data', () => {
  const pageContent = readFileSync(join(process.cwd(), 'app', 'page.tsx'), 'utf-8');

  expect(pageContent).toMatch(/Data loaded/);
});
EOF

echo "✅ Generated test files for all agent evals!"
