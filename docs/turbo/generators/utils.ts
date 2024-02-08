interface Answers {
  turboStars: string;
  turboDownloads: string;
  turboYearsSaved: string;
}

const MINUTES_IN_YEAR = 60 * 24 * 365;

const PUBLIC_TB_TOKEN =
  "p.eyJ1IjogIjAzYzA0Y2MyLTM1YTAtNDhhNC05ZTZjLThhMWE0NGNhNjhkZiIsICJpZCI6ICJmOWIzMTU5Yi0wOTVjLTQyM2UtOWIwNS04ZDZlNzIyNjEwNzIifQ.A3TOPdm3Lhmn-1x5m6jNvulCQbbgUeQfAIO3IaaAt5k";

export async function releasePostStats(answers: Answers): Promise<string> {
  const [starsResponse, downloadsResponse, timeSavedResponse] =
    await Promise.all([
      fetch("https://api.github.com/repos/vercel/turbo"),
      fetch("https://api.npmjs.org/versions/turbo/last-week"),
      fetch(
        `https://api.us-east.tinybird.co/v0/pipes/turborepo_time_saved_ticker.json?token=${PUBLIC_TB_TOKEN}`
      ),
    ]);

  const [starsData, downloadsData, timeSavedData] = await Promise.all([
    starsResponse.json() as unknown as { stargazers_count: number },
    downloadsResponse.json() as unknown as {
      downloads: Record<string, number>;
    },
    timeSavedResponse.json() as unknown as {
      data: [
        {
          remote_cache_minutes_saved: number;
          local_cache_minutes_saved: number;
        }
      ];
    },
  ]);

  const totalMinutesSaved: number =
    timeSavedData.data[0].remote_cache_minutes_saved +
    timeSavedData.data[0].local_cache_minutes_saved;
  const totalYearsSaved: number = Math.floor(
    totalMinutesSaved / MINUTES_IN_YEAR
  );
  const weeklyDownloads: number = Object.keys(
    downloadsData.downloads
  ).reduce<number>((sum, version) => sum + downloadsData.downloads[version], 0);

  const prettyRound = (num: number): string => {
    if (num < 1000) {
      return num.toString();
    } else if (num < 1000000) {
      return `${(num / 1000).toFixed(1)}k`;
    }
    return `${(num / 1000000).toFixed(1)}M`;
  };

  // extend answers
  answers.turboStars = prettyRound(starsData.stargazers_count);
  answers.turboDownloads = prettyRound(weeklyDownloads);
  answers.turboYearsSaved = prettyRound(totalYearsSaved);

  return "Fetched stats for release post";
}
