import { PolarAngleAxis, RadialBar, RadialBarChart } from "recharts";

import { TRACK_COLOR, bandColor, scoreBand } from "./chart-theme";

/**
 * A 0–100 score as a ring in its band's colour, with the number in the middle; "—" and an empty
 * ring when there is no score. The chart is decorative: the label carries the value for assistive
 * technology.
 */
export function ScoreGauge({ score, label, size = 96 }: { score: number | null; label: string; size?: number }) {
  const band = scoreBand(score);
  return (
    <div
      aria-label={score === null ? `${label}: not enough data` : `${label}: ${score} out of 100`}
      className="relative inline-flex shrink-0 items-center justify-center"
      data-band={band}
      role="img"
      style={{ height: size, width: size }}
    >
      <RadialBarChart
        aria-hidden
        barSize={Math.max(6, Math.round(size / 10))}
        data={[{ value: score ?? 0 }]}
        endAngle={-270}
        height={size}
        innerRadius="78%"
        outerRadius="100%"
        startAngle={90}
        width={size}
      >
        <PolarAngleAxis angleAxisId={0} domain={[0, 100]} tick={false} type="number" />
        <RadialBar
          angleAxisId={0}
          background={{ fill: TRACK_COLOR }}
          cornerRadius={size}
          dataKey="value"
          fill={bandColor(band)}
          isAnimationActive={false}
        />
      </RadialBarChart>
      <span className="absolute text-xl font-semibold tabular-nums text-foreground">{score ?? "—"}</span>
    </div>
  );
}
