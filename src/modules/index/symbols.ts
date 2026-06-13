import { invoke } from "@tauri-apps/api/core";

export type SymbolKind =
  | "function"
  | "class"
  | "method"
  | "interface"
  | "typeAlias"
  | "enum";

export interface FileSymbol {
  name: string;
  kind: SymbolKind;
  startLine: number;
  endLine: number;
}

export function listFileSymbols(path: string): Promise<FileSymbol[]> {
  return invoke<FileSymbol[]>("index_file_symbols", { path });
}
