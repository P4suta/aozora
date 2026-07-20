interface PerfBadgeProps {
  parseDurationMs: number;
  byteLen: number;
}

function fmtMs(ms: number): string {
  if (ms <= 0) return '—';
  if (ms < 1) return `${ms.toFixed(2)} ms`;
  if (ms < 10) return `${ms.toFixed(2)} ms`;
  return `${ms.toFixed(1)} ms`;
}

function fmtMbps(bytes: number, ms: number): string {
  if (ms <= 0 || bytes === 0) return '—';
  const mbps = bytes / 1_000_000 / (ms / 1000);
  if (mbps >= 100) return `${mbps.toFixed(0)} MB/s`;
  if (mbps >= 10) return `${mbps.toFixed(1)} MB/s`;
  return `${mbps.toFixed(2)} MB/s`;
}

function summary(props: PerfBadgeProps): string {
  if (props.byteLen === 0) return '—';
  return `${fmtMs(props.parseDurationMs)} · ${fmtMbps(props.byteLen, props.parseDurationMs)} · ${props.byteLen.toLocaleString()} bytes`;
}

export default function PerfBadge(props: PerfBadgeProps) {
  return (
    <span class="perf-badge" aria-label={summary(props)}>
      {summary(props)}
    </span>
  );
}
