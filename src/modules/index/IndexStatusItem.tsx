import { useIndexStore } from "./store";

export function IndexStatusItem() {
  const phase = useIndexStore((s) => s.phase);
  const indexed = useIndexStore((s) => s.indexed);
  const total = useIndexStore((s) => s.total);
  const symbolCount = useIndexStore((s) => s.symbolCount);

  if (phase === "idle") return null;

  const label =
    phase === "indexing"
      ? total > 0
        ? `Indexing… ${indexed}/${total}`
        : "Indexing…"
      : `Indexed · ${symbolCount.toLocaleString()} symbols`;

  return (
    <span className="flex shrink-0 cursor-default items-center gap-1 text-[10.5px] text-muted-foreground">
      {label}
    </span>
  );
}
