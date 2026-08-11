import { getRemoteCacheSavedMinutes } from "@/app/api/remote-cache-minutes-saved/route";
import { REMOTE_CACHE_COUNTER_START_HOURS } from "@/components/remote-cache-counter/constants";

const FALLBACK = {
  downloads: "17.5M",
  openIssues: "1",
  stars: "30.5K",
};

const integerFormatter = new Intl.NumberFormat("en-US", {
  maximumFractionDigits: 0,
});

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

const fetchHoursSaved = async (): Promise<string> => {
  try {
    const { total } = await getRemoteCacheSavedMinutes();
    return integerFormatter.format(total / 60);
  } catch {
    return integerFormatter.format(REMOTE_CACHE_COUNTER_START_HOURS);
  }
};

const fetchDownloads = async (): Promise<string> => {
  try {
    const response = await fetch(
      "https://api.npmjs.org/downloads/point/last-week/turbo",
      { next: { revalidate: 3600 } },
    );

    if (!response.ok) {
      return FALLBACK.downloads;
    }

    const data = (await response.json()) as { downloads: number };
    return formatNumber(data.downloads);
  } catch {
    return FALLBACK.downloads;
  }
};

const fetchRepositoryMetrics = async (): Promise<{
  stars: string;
}> => {
  try {
    const response = await fetch(
      "https://api.github.com/repos/vercel/turborepo",
      { next: { revalidate: 3600 } },
    );

    if (!response.ok) {
      return { stars: FALLBACK.stars };
    }

    const data = (await response.json()) as {
      stargazers_count: number;
    };

    return {
      stars: formatNumber(data.stargazers_count),
    };
  } catch {
    return { stars: FALLBACK.stars };
  }
};

const fetchOpenIssues = async (): Promise<string> => {
  try {
    const response = await fetch(
      "https://api.github.com/search/issues?q=repo%3Avercel%2Fturborepo+type%3Aissue+state%3Aopen",
      { next: { revalidate: 3600 } },
    );

    if (!response.ok) {
      return FALLBACK.openIssues;
    }

    const data = (await response.json()) as { total_count: number };
    return formatNumber(data.total_count);
  } catch {
    return FALLBACK.openIssues;
  }
};

export async function OpenSourceMetrics() {
  const [hoursSaved, downloads, repository, openIssues] = await Promise.all([
    fetchHoursSaved(),
    fetchDownloads(),
    fetchRepositoryMetrics(),
    fetchOpenIssues(),
  ]);

  const metrics = [
    { label: "Compute hours saved with Remote Caching", value: hoursSaved },
    { label: "Weekly downloads", value: downloads },
    { label: "GitHub stars", value: repository.stars },
    { label: "Open issue(s)", value: openIssues },
  ];

  return (
    <dl className="grid grid-cols-[1.6fr_repeat(3,minmax(0,1fr))] gap-x-3 gap-y-10 py-10 sm:gap-x-8 lg:gap-x-12 lg:py-12">
      {metrics.map((metric, index) => {
        const isFeatured = index === 0;

        return (
          <div
            className="min-w-0"
            key={metric.label}
          >
            <dt
              className={
                isFeatured
                  ? "inline-block bg-gradient-to-r from-[#FF1E56] to-[#0196FF] bg-clip-text pr-1 text-[clamp(1.25rem,5vw,4.5rem)] leading-none font-[450] tracking-[-0.04em] text-transparent"
                  : "text-gray-1000 text-[clamp(1rem,4vw,4rem)] leading-none font-[450] tracking-[-0.04em]"
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
