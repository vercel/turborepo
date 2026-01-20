"use client";

import { useEffect, useRef } from "react";
import { Chart, registerables } from "chart.js";

Chart.register(...registerables);

export default function Component() {
  const chartRef = useRef<HTMLCanvasElement | null>(null);
  const chartInstance = useRef<Chart | null>(null);

  useEffect(() => {
    if (chartRef.current) {
      // Destroy existing chart
      if (chartInstance.current) {
        chartInstance.current.destroy();
      }

      const ctx = chartRef.current.getContext("2d");
      if (!ctx) return;

      // Generate dates for the last 30 days
      const dates = Array.from({ length: 31 }, (_, i) => {
        const date = new Date();
        date.setDate(date.getDate() - 30 + i);
        return date.toLocaleDateString("en-US", {
          month: "short",
          day: "numeric"
        });
      });

      // Generate random data for different models
      const generateModelData = (baseValue: number, variance: number) => {
        return dates.map(() => baseValue + Math.random() * variance);
      };

      chartInstance.current = new Chart(ctx, {
        type: "line",
        data: {
          labels: dates,
          datasets: [
            {
              label: "XAI Grok 4",
              data: generateModelData(0.85, 0.15),
              borderColor: "rgb(99, 102, 241)",
              backgroundColor: "rgba(99, 102, 241, 0.1)",
              tension: 0.4,
              fill: true
            },
            {
              label: "Claude 3.7 Sonnet",
              data: generateModelData(0.82, 0.15),
              borderColor: "rgb(14, 165, 233)",
              backgroundColor: "rgba(14, 165, 233, 0.1)",
              tension: 0.4,
              fill: true
            },
            {
              label: "GPT-4o",
              data: generateModelData(0.8, 0.15),
              borderColor: "rgb(16, 185, 129)",
              backgroundColor: "rgba(16, 185, 129, 0.1)",
              tension: 0.4,
              fill: true
            }
          ]
        },
        options: {
          responsive: true,
          maintainAspectRatio: false,
          plugins: {
            legend: {
              position: "top"
            },
            tooltip: {
              mode: "index",
              intersect: false
            }
          },
          scales: {
            y: {
              min: 0,
              max: 1,
              title: {
                display: true,
                text: "Performance Score"
              }
            },
            x: {
              title: {
                display: true,
                text: "Date"
              }
            }
          },
          interaction: {
            mode: "nearest",
            axis: "x",
            intersect: false
          }
        }
      });
    }

    return () => {
      if (chartInstance.current) {
        chartInstance.current.destroy();
      }
    };
  }, []);

  return <canvas ref={chartRef} />;
}
