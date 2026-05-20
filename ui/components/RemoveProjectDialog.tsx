import { useState } from "react";
import type { ApiClient } from "../api";

export function RemoveProjectDialog({
  api,
  slug,
  name,
  onClose,
  onRemoved,
}: {
  api: ApiClient;
  slug: string;
  name: string;
  onClose: () => void;
  onRemoved: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const confirm = async () => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      const intent = await api.issueDeleteToken(slug);
      await api.removeProject(slug, intent.confirm_token);
      onRemoved();
      onClose();
    } catch (e) {
      setError((e as Error).message || "Lỗi không rõ");
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
      <div className="card" onClick={(e) => e.stopPropagation()} style={{ minWidth: 480 }}>
        <h3>Remove project?</h3>
        <p style={{ marginBottom: 16, color: "var(--subtle)" }}>
          Xóa index cache cho <b>{name}</b>. Source code không bị ảnh hưởng.
        </p>
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
            className="btn btn-primary"
            style={{ background: "var(--err)", borderColor: "var(--err)" }}
            onClick={confirm}
            disabled={busy}
          >
            {busy ? "Removing…" : "Remove"}
          </button>
        </div>
      </div>
    </div>
  );
}
