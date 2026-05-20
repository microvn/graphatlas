import type { ProjectRow } from "../types";
import { formatBytes, formatCount, formatRelativeTime } from "../formatters";
import { StateBadge, WatcherIndicator } from "./Badges";

export function ProjectsTable({
  rows,
  nowUnix,
  onSelect,
}: {
  rows: ProjectRow[];
  nowUnix: number;
  onSelect: (slug: string) => void;
}) {
  return (
    <div className="tbl-wrap">
      <table className="projects">
        <thead>
          <tr>
            <th colSpan={4}>Basic</th>
            <th colSpan={4} className="col-divider">Index size</th>
            <th colSpan={1} className="col-divider">Health</th>
            <th colSpan={2} className="col-divider">Watcher</th>
          </tr>
          <tr>
            <th>Name</th>
            <th>Languages</th>
            <th>Last indexed</th>
            <th>State</th>
            <th className="col-divider">Nodes</th>
            <th>Edges</th>
            <th>Files</th>
            <th>Size</th>
            <th className="col-divider">Hubs · Bridges · Dead · Large · Tested</th>
            <th className="col-divider">Status</th>
            <th>Queue</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => (
            <ProjectRowView
              key={row.slug}
              row={row}
              nowUnix={nowUnix}
              onSelect={onSelect}
            />
          ))}
        </tbody>
      </table>
    </div>
  );
}

function ProjectRowView({
  row,
  nowUnix,
  onSelect,
}: {
  row: ProjectRow;
  nowUnix: number;
  onSelect: (slug: string) => void;
}) {
  const isOrphan = row.index_state === "Orphan";
  const onClick = () => {
    if (isOrphan) return;
    onSelect(row.slug);
  };
  return (
    <tr onClick={onClick} className={isOrphan ? "dim" : undefined}>
      <td>
        <div className="proj-name">{row.name}</div>
        <div className={`proj-path${isOrphan ? " dim" : ""}`}>
          {row.repo_root}
          {isOrphan && <em> (path missing)</em>}
        </div>
      </td>
      <td>
        <span className="lang-list">
          {row.languages.length === 0 ? (
            <span className="empty-dash">—</span>
          ) : (
            row.languages.map((l) => (
              <span key={l.lang} className="lang">
                {l.lang}
                {l.file_count > 0 ? ` ${formatCount(l.file_count)}` : ""}
              </span>
            ))
          )}
        </span>
      </td>
      <td className="num">
        {formatRelativeTime(row.last_indexed_unix, nowUnix)}
      </td>
      <td>
        <StateBadge state={row.index_state} />
      </td>
      <td className="col-divider num">{formatCount(row.index_counts?.node_count)}</td>
      <td className="num">{formatCount(row.index_counts?.edge_count)}</td>
      <td className="num">{formatCount(row.index_counts?.file_count)}</td>
      <td className="num">{formatBytes(row.index_counts?.db_size_bytes ?? 0)}</td>
      <td className="col-divider health-cell">
        {row.health ? (
          <>
            <span className="h">
              <span className="v">{row.health.hubs_count}</span>
              <span className="l">hub</span>
            </span>
            <span className="h">
              <span className="v">{row.health.bridges_count}</span>
              <span className="l">brg</span>
            </span>
            <span className="h">
              <span className="v">{row.health.dead_code_count}</span>
              <span className="l">dead</span>
            </span>
            <span className="h">
              <span className="v">{row.health.large_functions_count}</span>
              <span className="l">large</span>
            </span>
            <span className="h">
              <span className="v">{row.health.tested_count}</span>
              <span className="l">test</span>
            </span>
          </>
        ) : (
          <span className="empty-dash">— reindex để cập nhật metrics</span>
        )}
      </td>
      <td className="col-divider">
        <WatcherIndicator status={row.watcher} />
      </td>
      <td className={`num${row.watcher_queue_pending === 0 ? " num-muted" : ""}`}>
        {row.watcher === "Stopped" || row.watcher === "Errored"
          ? "—"
          : formatCount(row.watcher_queue_pending)}
      </td>
    </tr>
  );
}
