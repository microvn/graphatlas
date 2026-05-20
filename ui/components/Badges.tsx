import type { ProjectIndexState, WatcherStatus } from "../types";
import {
  indexStateBadgeClass,
  indexStateLabel,
  watcherDotClass,
} from "../formatters";

export function StateBadge({ state }: { state: ProjectIndexState }) {
  return <span className={indexStateBadgeClass(state)}>{indexStateLabel(state)}</span>;
}

export function WatcherIndicator({
  status,
  errorReason,
}: {
  status: WatcherStatus;
  errorReason?: string | null;
}) {
  const label =
    status === "Running"
      ? "running"
      : status === "Errored"
        ? "errored"
        : "stopped";
  const style: React.CSSProperties =
    status === "Errored" ? { color: "var(--err)" } : status === "Stopped" ? { color: "var(--muted)" } : {};
  return (
    <span title={errorReason ?? undefined}>
      <span className={watcherDotClass(status)} />
      <span style={style}>{label}</span>
    </span>
  );
}
