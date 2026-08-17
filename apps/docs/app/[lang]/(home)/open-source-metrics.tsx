import { RemoteCacheCounterClient } from "@/components/remote-cache-counter/client";

const FALLBACK = {
  downloads: "17.5M",
  openIssues: 0,
  stars: "30.5K",
};

const formatNumber = (value: number): string => {
  if (value >= 1_000_000) {
    const millions = value / 1_000_000;
    return `${Number.parseFloat(millions.toFixed(1))}M`;
  }

  if (value >= 1_000) {
    const thousands = value / 1_000;
    return `${Number.parseFloat(thousands.toFixed(1))}K`;
  }

  return `${value}`;
};

const getOpenIssuesLabel = (count: number): string => {
  if (count === 1) {
    return "Open issue";
  }

  if (count > 1) {
    return "Open issues";
  }

  return "Open issue count";
};

const fetchDownloads = async (): Promise<string> => {
  const response = await fetch(
    "https://api.npmjs.org/downloads/point/last-week/turbo",
  );

  if (!response.ok) {
    throw new Error(`Failed to fetch npm downloads: ${response.status}`);
  }

  const data = (await response.json()) as { downloads: number };
  return formatNumber(data.downloads);
};

const fetchRepositoryMetrics = async (): Promise<{
  stars: string;
}> => {
  const response = await fetch(
    "https://api.github.com/repos/vercel/turborepo",
  );

  if (!response.ok) {
    throw new Error(`Failed to fetch GitHub stars: ${response.status}`);
  }

  const data = (await response.json()) as {
    stargazers_count: number;
  };

  return {
    stars: formatNumber(data.stargazers_count),
  };
};

const fetchOpenIssues = async (): Promise<number> => {
  const response = await fetch(
    "https://api.github.com/search/issues?q=repo%3Avercel%2Fturborepo+type%3Aissue+state%3Aopen",
  );

  if (!response.ok) {
    throw new Error(`Failed to fetch GitHub issues: ${response.status}`);
  }

  const data = (await response.json()) as { total_count: number };
  return data.total_count;
};

export async function OpenSourceMetrics() {
  const [downloads, repository, openIssues] = await Promise.all([
    fetchDownloads().catch(() => FALLBACK.downloads),
    fetchRepositoryMetrics().catch(() => ({ stars: FALLBACK.stars })),
    fetchOpenIssues().catch(() => FALLBACK.openIssues),
  ]);

  const metrics = [
    {
      id: "compute-hours",
      label: (
        <>
          Compute hours saved
          <span className="hidden lg:inline"> with Remote Caching</span>
        </>
      ),
      value: (
        <RemoteCacheCounterClient className="min-w-0 text-[inherit] leading-[inherit] font-[inherit] tracking-[inherit]" />
      ),
    },
    { id: "downloads", label: "Weekly downloads", value: downloads },
    { id: "stars", label: "GitHub stars", value: repository.stars },
    {
      id: "issues",
      label: getOpenIssuesLabel(openIssues),
      value: formatNumber(openIssues),
    },
  ];

  return (
    <dl className="grid grid-cols-2 gap-x-3 gap-y-10 py-10 sm:gap-x-8 lg:grid-cols-[1.6fr_repeat(3,minmax(0,1fr))] lg:gap-x-12 lg:py-12">
      {metrics.map((metric, index) => {
        const isFeatured = index === 0;

        return (
          <div className="min-w-0" key={metric.id}>
            <dt
              className={
                isFeatured
                  ? "text-heading-32 sm:text-heading-40 lg:text-heading-64 inline-block bg-gradient-to-r from-[#FF1E56] to-[#0196FF] bg-clip-text pr-1 text-transparent"
                  : "text-heading-32 sm:text-heading-40 lg:text-heading-64 text-gray-1000"
              }
            >
              {metric.value}
            </dt>
            <dd className="text-copy-16 text-gray-1000 sm:whitespace-nowrap">
              {metric.label}
            </dd>
          </div>
        );
      })}
    </dl>
  );
}
