import { useState } from "react";
import type { ApiClient } from "../api";
import type { AddProjectMode } from "../types";

export function AddProjectModal({
  api,
  onClose,
  onAdded,
}: {
  api: ApiClient;
  onClose: () => void;
  onAdded: () => void;
}) {
  const [path, setPath] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  // AS-021 — debounce double-click via the `busy` flag.
  const submit = async (mode: AddProjectMode) => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      await api.addProject(path, mode);
      onAdded();
      onClose();
    } catch (e) {
      const code = (e as { error?: string }).error;
      setError(mapAddError(code, (e as Error).message));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div
      style={{
        position: "fixed",
        inset: 0,
        background: "rgba(0,0,0,0.6)",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        zIndex: 50,
      }}
      onClick={onClose}
    >
      <div
        className="card"
        onClick={(e) => e.stopPropagation()}
        style={{ minWidth: 480, maxWidth: 640 }}
      >
        <h3>Add project</h3>
        <p style={{ color: "var(--muted)", fontSize: 12, marginBottom: 12 }}>
          Đường dẫn tuyệt đối tới repo. GraphAtlas sẽ chạy <code>ga reindex</code> nền.
        </p>
        <input
          className="search"
          style={{ width: "100%", marginBottom: 8 }}
          placeholder="/Users/you/work/some-repo"
          value={path}
          onChange={(e) => setPath(e.target.value)}
          disabled={busy}
          autoFocus
        />
        {error && (
          <div style={{ color: "var(--err)", fontSize: 12, marginBottom: 8 }}>
            {error}
          </div>
        )}
        <div style={{ display: "flex", gap: 8, justifyContent: "flex-end" }}>
          <button className="btn btn-ghost" onClick={onClose} disabled={busy}>
            Cancel
          </button>
          <button
            className="btn"
            onClick={() => submit("attach")}
            disabled={busy || !path.trim()}
            title="Use existing cache, no reindex"
          >
            Attach
          </button>
          <button
            className="btn btn-primary"
            onClick={() => submit("index")}
            disabled={busy || !path.trim()}
          >
            {busy ? "Indexing…" : "Index + Add"}
          </button>
        </div>
      </div>
    </div>
  );
}

function mapAddError(code: string | undefined, fallback: string): string {
  switch (code) {
    case "path_not_found":
      return "Path không tồn tại";
    case "path_not_directory":
      return "Path không phải thư mục";
    case "path_unsafe":
      return "Path không an toàn (có .. hoặc trỏ vào cache_root)";
    case "path_contains_external_symlink":
      return "Path chứa symlink trỏ ra ngoài";
    case "cache_not_found":
      return "Chưa có index cho path này — dùng Index + Add";
    case "reindex_in_progress":
      return "Đang reindex — chờ xong hoặc cancel";
    default:
      return fallback || "Lỗi không rõ — kiểm tra log ga-server";
  }
}
