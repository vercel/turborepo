const axios = require("axios");

const PUBLIC_TB_TOKEN =
  "p.eyJ1IjogIjAzYzA0Y2MyLTM1YTAtNDhhNC05ZTZjLThhMWE0NGNhNjhkZiIsICJpZCI6ICJmOWIzMTU5Yi0wOTVjLTQyM2UtOWIwNS04ZDZlNzIyNjEwNzIifQ.A3TOPdm3Lhmn-1x5m6jNvulCQbbgUeQfAIO3IaaAt5k";

const dateToday = () =>
  new Date().toISOString().split("T")[0].replace(/-/g, "/");
const majorMinor = (version) => version.split(".").slice(0, 2).join(".");

async function releasePostStats(answers) {
  const [starsResponse, downloadsResponse, timeSavedResponse] =
    await Promise.all([
      axios.get("https://api.github.com/repos/vercel/turbo"),
      axios.get("https://api.npmjs.org/versions/turbo/last-week"),
      axios.get(
        `https://api.us-east.tinybird.co/v0/pipes/turborepo_time_saved_ticker.json?token=${PUBLIC_TB_TOKEN}`
      ),
    ]);
  const stars = starsResponse.data.stargazers_count;
  const downloadsByVersion = downloadsResponse.data.downloads;
  const timeSavedData = timeSavedResponse.data.data[0];
  const totalMinutesSaved =
    timeSavedData.remote_cache_minutes_saved +
    timeSavedData.local_cache_minutes_saved;
  const totalYearsSaved = Math.floor(totalMinutesSaved / 60 / 24 / 365);
  const weeklyDownloads = Object.values(downloadsByVersion).reduce(
    (sum, a) => sum + a,
    0
  );

  const prettyRound = (num) => {
    if (num < 1000) {
      return num.toString();
    } else if (num < 1000000) {
      return (num / 1000).toFixed(1) + "k";
    } else {
      return (num / 1000000).toFixed(1) + "M";
    }
  };

  // extend answers
  answers.turboStars = prettyRound(stars);
  answers.turboDownloads = prettyRound(weeklyDownloads);
  answers.turboYearsSaved = prettyRound(totalYearsSaved);
}

module.exports = {
  releasePostStats,
  dateToday,
  majorMinor,
};
