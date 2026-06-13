use std::io::Write;

use terax_lib::modules::index::symbols::{index_file_symbols, SymbolKind};

#[test]
fn command_returns_symbols_for_a_file() {
    let mut file = tempfile::Builder::new()
        .suffix(".ts")
        .tempfile()
        .expect("create temp file");
    write!(file, "export function run() {{}}\nclass Engine {{}}\n").expect("write source");

    let path = file.path().to_string_lossy().to_string();
    let symbols = index_file_symbols(path).expect("command should succeed");

    let pairs: Vec<(&str, &SymbolKind)> =
        symbols.iter().map(|s| (s.name.as_str(), &s.kind)).collect();
    assert_eq!(
        pairs,
        vec![
            ("run", &SymbolKind::Function),
            ("Engine", &SymbolKind::Class),
        ]
    );
}

#[test]
fn command_errors_on_missing_file() {
    let result = index_file_symbols("/no/such/ken/file.ts".to_string());
    assert!(result.is_err());
}
