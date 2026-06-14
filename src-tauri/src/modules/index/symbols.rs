use serde::Serialize;
use tree_sitter::{Node, Parser};

#[derive(Serialize, Clone, Debug, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SymbolKind {
    Function,
    Class,
    Method,
    Interface,
    TypeAlias,
    Enum,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub start_line: usize,
    pub end_line: usize,
}

fn node_name(node: Node, source: &[u8]) -> Option<String> {
    let name_node = node.child_by_field_name("name")?;
    name_node
        .utf8_text(source)
        .ok()
        .map(|s| s.to_string())
}

fn kind_for(node_kind: &str) -> Option<SymbolKind> {
    match node_kind {
        "function_declaration" => Some(SymbolKind::Function),
        "class_declaration" => Some(SymbolKind::Class),
        "method_definition" => Some(SymbolKind::Method),
        "interface_declaration" => Some(SymbolKind::Interface),
        "type_alias_declaration" => Some(SymbolKind::TypeAlias),
        "enum_declaration" => Some(SymbolKind::Enum),
        _ => None,
    }
}

fn walk(node: Node, source: &[u8], out: &mut Vec<Symbol>) {
    if let Some(kind) = kind_for(node.kind()) {
        if let Some(name) = node_name(node, source) {
            out.push(Symbol {
                name,
                kind,
                start_line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
            });
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, source, out);
    }
}

pub fn extract_symbols(source: &str) -> Vec<Symbol> {
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .is_err()
    {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    walk(tree.root_node(), bytes, &mut out);
    out
}

#[tauri::command]
pub fn index_file_symbols(path: String) -> Result<Vec<Symbol>, String> {
    let source = std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))?;
    Ok(extract_symbols(&source))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_functions_and_classes() {
        let src = "function greet(name: string) {\n  return name;\n}\n\nclass Service {\n}\n";
        let symbols = extract_symbols(src);
        assert_eq!(
            symbols,
            vec![
                Symbol {
                    name: "greet".to_string(),
                    kind: SymbolKind::Function,
                    start_line: 1,
                    end_line: 3,
                },
                Symbol {
                    name: "Service".to_string(),
                    kind: SymbolKind::Class,
                    start_line: 5,
                    end_line: 6,
                },
            ]
        );
    }

    #[test]
    fn extracts_methods_interfaces_type_aliases_and_enums() {
        let src = concat!(
            "interface Repo {\n",
            "  id: number;\n",
            "}\n",
            "type Id = string;\n",
            "enum Color {\n",
            "  Red,\n",
            "}\n",
            "class Store {\n",
            "  save() {}\n",
            "}\n",
        );
        let symbols = extract_symbols(src);
        let pairs: Vec<(&str, &SymbolKind)> =
            symbols.iter().map(|s| (s.name.as_str(), &s.kind)).collect();
        assert_eq!(
            pairs,
            vec![
                ("Repo", &SymbolKind::Interface),
                ("Id", &SymbolKind::TypeAlias),
                ("Color", &SymbolKind::Enum),
                ("Store", &SymbolKind::Class),
                ("save", &SymbolKind::Method),
            ]
        );
    }
}
