// Spec A response shapes mirrored. Keep in sync with
// crates/ga-server/src/handlers/projects_types.rs.

export type ProjectIndexState =
  | "Fresh"
  | "Building"
  | "Orphan"
  | "Corrupt"
  | "Stale";

export type WatcherStatus = "Running" | "Stopped" | "Errored";

export interface LangCount {
  lang: string;
  file_count: number;
}

export interface IndexCounts {
  node_count: number;
  edge_count: number;
  file_count: number;
  last_index_duration_ms: number;
  db_size_bytes: number;
}

export interface HealthSummary {
  computed_at_unix: number;
  hubs_count: number;
  bridges_count: number;
  dead_code_count: number;
  large_functions_count: number;
  tested_count: number;
}

export interface ProjectRow {
  slug: string;
  name: string;
  repo_root: string;
  languages: LangCount[];
  last_indexed_unix: number;
  index_state: ProjectIndexState;
  index_counts: IndexCounts | null;
  health: HealthSummary | null;
  watcher: WatcherStatus;
  watcher_queue_pending: number;
  watcher_last_event_unix: number | null;
}

export type AddProjectMode = "index" | "attach";

export interface AddProjectResponse {
  slug: string;
  job_id: string | null;
  mode: string;
  canonical_path: string;
}

export interface DeleteIntentResponse {
  confirm_token: string;
  expires_in_secs: number;
}

export interface ApiError {
  error: string;
  message: string;
}

export class SessionExpiredError extends Error {
  constructor() {
    super("session token rejected");
    this.name = "SessionExpiredError";
  }
}

export class NetworkError extends Error {
  constructor(cause: unknown) {
    super(`network error: ${String(cause)}`);
    this.name = "NetworkError";
  }
}

export class CacheCorruptError extends Error {
  constructor() {
    super("cache corrupt — reindex required");
    this.name = "CacheCorruptError";
  }
}

export class CacheBuildingError extends Error {
  constructor() {
    super("cache building — retry after reindex completes");
    this.name = "CacheBuildingError";
  }
}

// ============== Spec A response shapes (mirror) ==============

export interface GraphNode {
  id: string;
  name: string;
  kind: string;
  file: string;
  line: number;
  line_end?: number | null;
  degree: number;
}

export interface GraphEdge {
  from: string;
  to: string;
  kind: string;
  line?: number | null;
}

export interface GraphResponse {
  nodes: GraphNode[];
  edges: GraphEdge[];
  truncated: boolean;
  total_node_count: number;
}

export interface ParamSlotDto {
  name: string;
  type: string;
  default_value: string;
}

export interface SymbolDetail {
  id: string;
  name: string;
  kind: string;
  file: string;
  line: number;
  line_end?: number | null;
  qualified_name?: string | null;
  rendered_signature: string;
  layer?: string | null;
  // Spec E S-003 extension
  loc?: number | null;
  doc_summary?: string | null;
  has_doc?: boolean;
  is_async?: boolean;
  is_abstract?: boolean;
  is_static?: boolean;
  is_override?: boolean;
  confidence?: number;
  is_dead_code?: boolean;
  is_hub?: boolean;
  tested?: boolean;
  caller_count?: number;
  callee_count?: number;
  importer_count?: number;
  impact_edge_count?: number;
  // S-004: null = STRUCT[] decoder degraded; [] = arity 0
  params?: ParamSlotDto[] | null;
}

// ============== Spec E S-001/S-002 ==============

export interface SymbolHit {
  id: string;
  name: string;
  kind: string;
  file: string;
  line: number;
  layer?: string | null;
}

export interface SymbolSearchResponse {
  hits: SymbolHit[];
  truncated: boolean;
}

export interface LayerEntry {
  name: string;
  symbol_count: number;
}

export interface LayersResponse {
  layers: LayerEntry[];
  degraded: boolean;
}

export interface LayerSymbolsResponse {
  symbols: SymbolHit[];
  symbol_ids: string[];
}

export class BadPatternError extends Error {
  constructor() {
    super("Ký tự không hợp lệ trong tìm kiếm");
    this.name = "BadPatternError";
  }
}

export interface RelationEntry {
  id: string;
  name: string;
  file: string;
  line: number;
  kind: string;
}

export interface RelationPage {
  entries: RelationEntry[];
  total: number;
  has_more: boolean;
  offset: number;
  limit: number;
}

export interface FileSummaryDetail {
  path: string;
  language: string | null;
  line_count: number | null;
  symbols: RelationEntry[];
  imports: string[];
  reverse_imports: string[];
}

export type JobState = "Running" | "Done" | "Error" | "Cancelled";

export interface ReindexStartResponse {
  slug: string;
  job_id: string;
}

export interface ReindexStatus {
  job_id: string;
  slug: string;
  state: JobState;
  percent: number;
  phase: string | null;
  current_file: string | null;
  files_done: number;
  files_total: number;
  duration_ms: number;
  error: string | null;
  log_tail: string[];
}

export type WatcherDriverStatus = "Running" | "Stopped" | "Errored";

export interface WatcherSnapshot {
  slug: string;
  status: WatcherDriverStatus;
  mode: "inotify" | "fsevents" | "rdcw" | "native" | "poll";
  queue_pending: number;
  dirty_flag: boolean;
  last_event_unix: number | null;
  error: string | null;
}
