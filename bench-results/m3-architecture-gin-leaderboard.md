# M3 Gate — `architecture` on `gin`

**Rule:** Ha-import-edge

**Policy bias:** Ha-import-edge — primary metric: F1 on edge-pairs (Spearman rank correlation utility not yet wired; promoted from EXP follow-up in Phase 3). Marker-based module GT (kind=module) is tautological-by-design vs ga_architecture::discover_modules — tracked as DIAGNOSTIC only, never as the primary spec gate. Module = dir-basename of nearest ancestor with `__init__.py` / `Cargo.toml` / `package.json` marker (mirrors ga_architecture::dir_basename for definitional alignment per spec C2). Files outside any marked dir are dropped. Cross-module edge is `(module_of_importer, module_of_imported)`; self-edges excluded. Import target → owning module via root-prefix match (longest wins) on the import's parsed target_path; unresolved imports (external packages, relative paths) are dropped.

| Status | Retriever | Score | Spec target | p95 latency (ms) |
|---|---|---|---|---|
| _no rows_ | — | — | — | — |
**SPEC GATE: 0 pass, 0 fail (target: all pass)**
