use tree_sitter::{Node, Parser, Tree};
use wae_core::domain::{
    Import, ImportKind, ModuleId, ModulePath, ParseError, ParseErrorKind, SourceLocation,
};

mod dependency_classifier;
use dependency_classifier::{classify_export, classify_import};

/// Increment the explicit suffix when parser behavior or grammar inputs change. Cache consumers
/// persist this value so parser upgrades can never reuse stale import IR.
pub const PARSER_CACHE_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), ":js-ts-ast-v3");

pub trait ParserAdapter: Send + Sync {
    fn parse_imports(
        &self,
        module_path: &ModulePath,
        source: &str,
    ) -> Result<Vec<Import>, ParseError>;
}

/// Dependency-oriented JS/TS adapter backed entirely by the Tree-sitter AST.
#[derive(Debug, Default)]
pub struct JsTsParser;

impl ParserAdapter for JsTsParser {
    fn parse_imports(
        &self,
        module_path: &ModulePath,
        source: &str,
    ) -> Result<Vec<Import>, ParseError> {
        let tree = parse_tree(module_path, source)?;
        let mut imports = Vec::new();
        collect_dependencies(tree.root_node(), module_path, source, &mut imports);
        imports.sort_by_key(|import| {
            (import.location.line, import.location.column, import.specifier.clone())
        });
        imports.dedup_by(|a, b| {
            a.specifier == b.specifier && a.location == b.location && a.kind == b.kind
        });
        Ok(imports)
    }
}

fn parse_tree(module_path: &ModulePath, source: &str) -> Result<Tree, ParseError> {
    let mut parser = Parser::new();
    let tsx = matches!(
        std::path::Path::new(&module_path.0).extension().and_then(|value| value.to_str()),
        Some("tsx" | "jsx")
    );
    let language = if tsx {
        tree_sitter_typescript::LANGUAGE_TSX
    } else {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT
    };
    parser.set_language(&language.into()).map_err(|error| ParseError {
        kind: ParseErrorKind::ProviderFailure,
        message: error.to_string(),
        location: None,
    })?;
    let tree = parser.parse(source, None).ok_or_else(|| ParseError {
        kind: ParseErrorKind::ProviderFailure,
        message: "parser did not produce a syntax tree".into(),
        location: None,
    })?;
    if tree.root_node().has_error() && !valid_with_import_attributes(&mut parser, source) {
        let error_node = first_error(tree.root_node());
        return Err(ParseError {
            kind: ParseErrorKind::MalformedSource,
            message: "malformed JavaScript/TypeScript source".into(),
            location: error_node
                .map(|node| source_location(module_path, source, node.start_byte())),
        });
    }
    Ok(tree)
}

fn valid_with_import_attributes(parser: &mut Parser, source: &str) -> bool {
    if !source.contains(" with {") {
        return false;
    }
    // v0.23 of the upstream TypeScript grammar recognizes the legacy `assert`
    // spelling but not its standards-track `with` replacement. This secondary
    // parse validates the equivalent grammar while extraction still uses the
    // original tree, preserving exact source positions.
    let compatible = source.replace(" with {", " assert {");
    parser.parse(&compatible, None).is_some_and(|tree| !tree.root_node().has_error())
}

fn first_error(node: Node<'_>) -> Option<Node<'_>> {
    if node.is_error() || node.is_missing() {
        return Some(node);
    }
    let mut cursor = node.walk();
    node.children(&mut cursor).find_map(first_error)
}

fn collect_dependencies(
    node: Node<'_>,
    module_path: &ModulePath,
    source: &str,
    output: &mut Vec<Import>,
) {
    match node.kind() {
        "import_statement" => {
            if let Some(specifier) = node.child_by_field_name("source") {
                let kind = classify_import(node, source);
                push_string_import(output, module_path, source, specifier, kind);
            }
            return;
        }
        "export_statement" => {
            if let Some(specifier) = node.child_by_field_name("source") {
                push_string_import(
                    output,
                    module_path,
                    source,
                    specifier,
                    classify_export(node, source),
                );
            }
            return;
        }
        "call_expression" => collect_call(node, module_path, source, output),
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_dependencies(child, module_path, source, output);
    }
}

fn collect_call(node: Node<'_>, module_path: &ModulePath, source: &str, output: &mut Vec<Import>) {
    let Some(function) = node.child_by_field_name("function") else { return };
    let kind = match function.utf8_text(source.as_bytes()).ok() {
        Some("import") => ImportKind::Dynamic,
        Some("require") if function.kind() == "identifier" => ImportKind::Require,
        _ => return,
    };
    let Some(arguments) = node.child_by_field_name("arguments") else { return };
    let mut cursor = arguments.walk();
    let mut named = arguments.named_children(&mut cursor);
    let Some(argument) = named.next() else { return };
    // Only literal module specifiers are statically resolvable. Expressions and
    // template substitutions intentionally do not become graph edges.
    if named.next().is_none() && argument.kind() == "string" {
        push_string_import(output, module_path, source, argument, kind);
    }
}

fn push_string_import(
    output: &mut Vec<Import>,
    module_path: &ModulePath,
    source: &str,
    string_node: Node<'_>,
    kind: ImportKind,
) {
    let Ok(raw) = string_node.utf8_text(source.as_bytes()) else { return };
    let Some(specifier) = decode_string_literal(raw) else { return };
    output.push(Import {
        module_id: ModuleId(module_path.0.clone()),
        specifier,
        kind,
        location: source_location(module_path, source, string_node.start_byte() + 1),
    });
}

fn source_location(module_path: &ModulePath, source: &str, byte_offset: usize) -> SourceLocation {
    let prefix = &source[..byte_offset.min(source.len())];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix
        .rsplit_once('\n')
        .map_or_else(|| prefix.chars().count() + 1, |(_, tail)| tail.chars().count() + 1);
    SourceLocation { file: module_path.0.clone(), line, column }
}

fn decode_string_literal(raw: &str) -> Option<String> {
    let quote = raw.as_bytes().first().copied()?;
    if raw.as_bytes().last().copied()? != quote || !matches!(quote, b'\'' | b'"') {
        return None;
    }
    let content = &raw[1..raw.len() - 1];
    if quote == b'"' {
        serde_json::from_str(raw).ok()
    } else {
        Some(
            content
                .replace("\\'", "'")
                .replace("\\\\", "\\")
                .replace("\\n", "\n")
                .replace("\\r", "\r")
                .replace("\\t", "\t"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_supported_dependency_nodes_from_the_ast() {
        let source = r#"
import value from './static';
import type { T } from "./types";
export * from './reexport';
export { value as other } from "./named";
import data from "./data.json" assert { type: "json" };
import modern from "./modern.json" with { type: "json" };
const lazy = import('./lazy');
const legacy = require('./legacy');
// import nope from './comment';
"#;
        let imports = JsTsParser.parse_imports(&ModulePath("src/a.ts".into()), source).unwrap();
        assert_eq!(
            imports.iter().map(|i| i.specifier.as_str()).collect::<Vec<_>>(),
            vec![
                "./static",
                "./types",
                "./reexport",
                "./named",
                "./data.json",
                "./modern.json",
                "./lazy",
                "./legacy",
            ]
        );
        assert_eq!(imports[1].kind, ImportKind::TypeOnly);
        assert_eq!(imports[2].kind, ImportKind::ReExport);
    }

    #[test]
    fn classifies_type_only_import_and_export_forms_without_erasing_mixed_runtime_edges() {
        let source = r#"
export type { User } from "./export-type";
export { type Account } from "./export-inline-type";
export { type Role, runtimeValue } from "./export-mixed";
import { type Session } from "./import-inline-type";
import { type Token, runtimeClient } from "./import-mixed";
import defaultValue, { type Options } from "./import-default-mixed";
"#;
        let imports = JsTsParser.parse_imports(&ModulePath("src/a.ts".into()), source).unwrap();
        let kinds = imports
            .iter()
            .map(|import| (import.specifier.as_str(), import.kind.clone()))
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(kinds["./export-type"], ImportKind::TypeOnly);
        assert_eq!(kinds["./export-inline-type"], ImportKind::TypeOnly);
        assert_eq!(kinds["./export-mixed"], ImportKind::ReExport);
        assert_eq!(kinds["./import-inline-type"], ImportKind::TypeOnly);
        assert_eq!(kinds["./import-mixed"], ImportKind::Static);
        assert_eq!(kinds["./import-default-mixed"], ImportKind::Static);
    }

    #[test]
    fn ignores_dependency_text_in_strings_templates_regex_and_member_calls() {
        let source = r#"
const text = "import value from './fake-string'";
const template = `require('./fake-template') ${value}`;
const regex = /import.*fake-regex/;
loader.require('./fake-member');
const expression = import(`./locale/${locale}`);
"#;
        let imports = JsTsParser.parse_imports(&ModulePath("src/a.ts".into()), source).unwrap();
        assert!(imports.is_empty());
    }

    #[test]
    fn handles_multiline_tsx_and_literal_dynamic_imports() {
        let source = r#"
import {
  Button,
} from './button';
const view = <Button />;
const page = import(
  "./page"
);
"#;
        let imports = JsTsParser.parse_imports(&ModulePath("src/view.tsx".into()), source).unwrap();
        assert_eq!(imports.len(), 2);
        assert_eq!(imports[0].specifier, "./button");
        assert_eq!(imports[1].specifier, "./page");
    }

    #[test]
    fn reports_malformed_typescript_from_the_syntax_tree() {
        let error = JsTsParser
            .parse_imports(&ModulePath("src/broken.ts".into()), "export const value = ;")
            .unwrap_err();
        assert_eq!(error.kind, ParseErrorKind::MalformedSource);
        assert_eq!(error.location.unwrap().line, 1);
    }

    #[test]
    fn reports_unicode_columns_in_characters_not_utf8_bytes() {
        let source = "const label = 'سلام'; const page = import('./page');";
        let imports = JsTsParser.parse_imports(&ModulePath("src/a.ts".into()), source).unwrap();
        let byte = source.find("./page").unwrap();
        assert_eq!(imports[0].location.column, source[..byte].chars().count() + 1);
    }
}
