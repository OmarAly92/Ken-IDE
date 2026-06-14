use std::collections::BTreeMap;
use std::sync::Mutex;

use serde::Serialize;

use crate::modules::fs::search::fuzzy_rank;
use crate::modules::index::symbols::{Symbol, SymbolKind};

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SymbolHit {
    pub name: String,
    pub kind: SymbolKind,
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IndexStatus {
    pub root: Option<String>,
    pub file_count: usize,
    pub symbol_count: usize,
}

#[derive(Default)]
struct IndexData {
    root: Option<String>,
    files: BTreeMap<String, Vec<Symbol>>,
}

#[derive(Default)]
pub struct IndexStore {
    inner: Mutex<IndexData>,
}

impl IndexStore {
    pub fn clear(&self) {
        let mut data = self.inner.lock().expect("index store poisoned");
        data.root = None;
        data.files.clear();
    }

    pub fn set_root(&self, root: Option<String>) {
        self.inner.lock().expect("index store poisoned").root = root;
    }

    pub fn replace_file(&self, path: String, symbols: Vec<Symbol>) {
        self.inner
            .lock()
            .expect("index store poisoned")
            .files
            .insert(path, symbols);
    }

    pub fn remove_file(&self, path: &str) {
        self.inner
            .lock()
            .expect("index store poisoned")
            .files
            .remove(path);
    }

    pub fn status(&self) -> IndexStatus {
        let data = self.inner.lock().expect("index store poisoned");
        IndexStatus {
            root: data.root.clone(),
            file_count: data.files.len(),
            symbol_count: data.files.values().map(|v| v.len()).sum(),
        }
    }

    pub fn query(&self, query: &str, limit: usize) -> Vec<SymbolHit> {
        let data = self.inner.lock().expect("index store poisoned");
        let mut entries: Vec<(&str, &Symbol)> = Vec::new();
        for (path, syms) in data.files.iter() {
            for s in syms {
                entries.push((path.as_str(), s));
            }
        }
        let make = |(path, s): (&str, &Symbol)| SymbolHit {
            name: s.name.clone(),
            kind: s.kind.clone(),
            path: path.to_string(),
            start_line: s.start_line,
            end_line: s.end_line,
        };
        if query.trim().is_empty() {
            return entries.into_iter().take(limit).map(make).collect();
        }
        let names: Vec<&str> = entries.iter().map(|(_, s)| s.name.as_str()).collect();
        fuzzy_rank(query, &names, limit)
            .into_iter()
            .map(|i| make(entries[i]))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sym(name: &str, line: usize) -> Symbol {
        Symbol {
            name: name.to_string(),
            kind: SymbolKind::Function,
            start_line: line,
            end_line: line,
        }
    }

    #[test]
    fn empty_store_reports_zero_and_returns_no_hits() {
        let store = IndexStore::default();
        let status = store.status();
        assert_eq!(status.file_count, 0);
        assert_eq!(status.symbol_count, 0);
        assert_eq!(status.root, None);
        assert!(store.query("anything", 10).is_empty());
    }

    #[test]
    fn replace_file_then_query_finds_symbol() {
        let store = IndexStore::default();
        store.replace_file("a.ts".to_string(), vec![sym("greet", 1), sym("run", 5)]);
        let hits = store.query("greet", 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "greet");
        assert_eq!(hits[0].path, "a.ts");
        assert_eq!(hits[0].start_line, 1);
        let status = store.status();
        assert_eq!(status.file_count, 1);
        assert_eq!(status.symbol_count, 2);
    }

    #[test]
    fn replace_file_overwrites_previous_symbols_for_that_path() {
        let store = IndexStore::default();
        store.replace_file("a.ts".to_string(), vec![sym("old", 1)]);
        store.replace_file("a.ts".to_string(), vec![sym("new", 2)]);
        assert_eq!(store.status().symbol_count, 1);
        assert!(store.query("old", 10).is_empty());
        assert_eq!(store.query("new", 10).len(), 1);
    }

    #[test]
    fn remove_file_drops_its_symbols() {
        let store = IndexStore::default();
        store.replace_file("a.ts".to_string(), vec![sym("greet", 1)]);
        store.remove_file("a.ts");
        assert_eq!(store.status().file_count, 0);
        assert!(store.query("greet", 10).is_empty());
    }

    #[test]
    fn query_respects_limit() {
        let store = IndexStore::default();
        store.replace_file(
            "a.ts".to_string(),
            vec![sym("handle", 1), sym("handler", 2), sym("handlers", 3)],
        );
        assert_eq!(store.query("handle", 2).len(), 2);
    }

    #[test]
    fn clear_resets_root_and_files() {
        let store = IndexStore::default();
        store.set_root(Some("/proj".to_string()));
        store.replace_file("a.ts".to_string(), vec![sym("greet", 1)]);
        store.clear();
        let status = store.status();
        assert_eq!(status.root, None);
        assert_eq!(status.file_count, 0);
    }
}
