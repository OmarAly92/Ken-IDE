import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import type { SymbolKind } from "./symbols";

export interface SymbolHit {
  name: string;
  kind: SymbolKind;
  path: string;
  startLine: number;
  endLine: number;
}

export interface IndexStatus {
  root: string | null;
  fileCount: number;
  symbolCount: number;
}

export interface IndexProgress {
  indexed: number;
  total: number;
}

export function indexProject(root: string): Promise<void> {
  return invoke<void>("index_project", { root });
}

export function querySymbols(query: string, limit?: number): Promise<SymbolHit[]> {
  return invoke<SymbolHit[]>("query_symbols", { query, limit });
}

export function indexStatus(): Promise<IndexStatus> {
  return invoke<IndexStatus>("index_status");
}

export function listenIndexProgress(
  handler: (p: IndexProgress) => void,
): Promise<() => void> {
  return getCurrentWebviewWindow().listen<IndexProgress>(
    "index:progress",
    (e) => handler(e.payload),
  );
}

export function listenIndexDone(
  handler: (s: IndexStatus) => void,
): Promise<() => void> {
  return getCurrentWebviewWindow().listen<IndexStatus>("index:done", (e) =>
    handler(e.payload),
  );
}

export function listenIndexUpdated(
  handler: (paths: string[]) => void,
): Promise<() => void> {
  return getCurrentWebviewWindow().listen<{ paths: string[] }>(
    "index:updated",
    (e) => handler(e.payload.paths),
  );
}
