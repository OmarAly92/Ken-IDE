import { create } from "zustand";

export type IndexPhase = "idle" | "indexing" | "ready";

interface IndexState {
  phase: IndexPhase;
  indexed: number;
  total: number;
  fileCount: number;
  symbolCount: number;
  startIndexing: () => void;
  setProgress: (indexed: number, total: number) => void;
  setReady: (fileCount: number, symbolCount: number) => void;
}

export const useIndexStore = create<IndexState>((set) => ({
  phase: "idle",
  indexed: 0,
  total: 0,
  fileCount: 0,
  symbolCount: 0,
  startIndexing: () => set({ phase: "indexing", indexed: 0, total: 0 }),
  setProgress: (indexed, total) => set({ phase: "indexing", indexed, total }),
  setReady: (fileCount, symbolCount) =>
    set({ phase: "ready", fileCount, symbolCount }),
}));
