import { useEffect } from "react";
import {
  indexProject,
  listenIndexDone,
  listenIndexProgress,
  listenIndexUpdated,
  indexStatus,
} from "./project";
import { useIndexStore } from "./store";

export function useProjectIndex(root: string | null): void {
  const startIndexing = useIndexStore((s) => s.startIndexing);
  const setProgress = useIndexStore((s) => s.setProgress);
  const setReady = useIndexStore((s) => s.setReady);

  useEffect(() => {
    const unlisteners: Array<() => void> = [];
    let active = true;
    void listenIndexProgress((p) => setProgress(p.indexed, p.total)).then((u) => {
      if (active) unlisteners.push(u);
      else u();
    });
    void listenIndexDone((s) => setReady(s.fileCount, s.symbolCount)).then((u) => {
      if (active) unlisteners.push(u);
      else u();
    });
    void listenIndexUpdated(() => {
      void indexStatus()
        .then((s) => setReady(s.fileCount, s.symbolCount))
        .catch(() => {});
    }).then((u) => {
      if (active) unlisteners.push(u);
      else u();
    });
    return () => {
      active = false;
      for (const u of unlisteners) u();
    };
  }, [setProgress, setReady]);

  useEffect(() => {
    if (!root) return;
    startIndexing();
    void indexProject(root).catch(() => {});
  }, [root, startIndexing]);
}
