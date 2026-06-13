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
  start_line: number;
  end_line: number;
}

export function listFileSymbols(path: string): Promise<FileSymbol[]> {
  return invoke<FileSymbol[]>("index_file_symbols", { path });
}
