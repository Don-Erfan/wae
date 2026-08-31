use tree_sitter::{Node, Parser, Tree};
use wae_core::domain::{
    Import, ImportKind, ModuleId, ModulePath, ModuleSemantics, ParseError, ParseErrorKind,
    SourceLocation,
};

mod dependency_classifier;
use dependency_classifier::{classify_export, classify_import};

/// Increment the explicit suffix when parser behavior or grammar inputs change. Cache consumers
/// persist this value so parser upgrades can never reuse stale import IR.
pub const PARSER_CACHE_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), ":js-ts-ast-v5");

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParsedModule {
    pub imports: Vec<Import>,
    pub semantics: ModuleSemantics,
}

pub trait ParserAdapter: Send + Sync {
    fn parse_module(
        &self,
        module_path: &ModulePath,
        source: &str,
    ) -> Result<ParsedModule, ParseError>;

    fn parse_imports(
        &self,
        module_path: &ModulePath,
        source: &str,
    ) -> Result<Vec<Import>, ParseError> {
        self.parse_module(module_path, source).map(|module| module.imports)
    }
}

/// Dependency-oriented JS/TS adapter backed entirely by the Tree-sitter AST.
#[derive(Debug, Default)]
pub struct JsTsParser;

impl ParserAdapter for JsTsParser {
    fn parse_module(
        &self,
        module_path: &ModulePath,
        source: &str,
    ) -> Result<ParsedModule, ParseError> {
        let tree = parse_tree(module_path, source)?;
        let mut imports = Vec::new();
        collect_dependencies(tree.root_node(), module_path, source, &mut imports);
        imports.sort_by_key(|import| {
            (import.location.line, import.location.column, import.specifier.clone())
        });
        imports.dedup_by(|a, b| {
            a.specifier == b.specifier && a.location == b.location && a.kind == b.kind
        });
        Ok(ParsedModule { imports, semantics: collect_semantics(tree.root_node(), source) })
    }
}

fn collect_semantics(root: Node<'_>, source: &str) -> ModuleSemantics {
    let mut semantics = ModuleSemantics::default();
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        if child.kind() == "comment" {
            continue;
        }
        if child.kind() == "expression_statement" {
            let value = child.named_child(0).filter(|node| node.kind() == "string");
            if let Some(directive) = value
                .and_then(|node| node.utf8_text(source.as_bytes()).ok())
                .and_then(decode_string_literal)
            {
                semantics.directives.push(directive);
                continue;
            }
        }
        break;
    }
    semantics.exported_runtime = find_exported_runtime(root, source);
    semantics
}

fn find_exported_runtime(root: Node<'_>, source: &str) -> Option<String> {
    // Next.js only recognizes a static export in the module body. Namespace/block/function
    // descendants and re-exports must not classify the containing module's runtime.
    let mut cursor = root.walk();
    root.named_children(&mut cursor)
        .filter(|child| child.kind() == "export_statement")
        .find_map(|child| runtime_declaration(child, source))
}

fn runtime_declaration(node: Node<'_>, source: &str) -> Option<String> {
    let declarator = first_descendant_of_kind(node, "variable_declarator")?;
    let name = declarator.child_by_field_name("name")?.utf8_text(source.as_bytes()).ok()?;
    if name != "runtime" {
        return None;
    }
    let value = declarator.child_by_field_name("value")?;
    (value.kind() == "string")
        .then(|| value.utf8_text(source.as_bytes()).ok().and_then(decode_string_literal))
        .flatten()
        .filter(|runtime| matches!(runtime.as_str(), "edge" | "nodejs"))
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
            } else {
                collect_import_equals_require(node, module_path, source, output);
            }
            return;
        }
        "import_alias" => {
            collect_import_equals_require(node, module_path, source, output);
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

fn collect_import_equals_require(
    node: Node<'_>,
    module_path: &ModulePath,
    source: &str,
    output: &mut Vec<Import>,
) {
    if !node.utf8_text(source.as_bytes()).ok().is_some_and(|text| text.contains("require")) {
        return;
    }
    if let Some(specifier) = first_descendant_of_kind(node, "string") {
        push_string_import(output, module_path, source, specifier, ImportKind::Require);
    }
}

fn first_descendant_of_kind<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    if node.kind() == kind {
        return Some(node);
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor).find_map(|child| first_descendant_of_kind(child, kind))
}

fn collect_call(node: Node<'_>, module_path: &ModulePath, source: &str, output: &mut Vec<Import>) {
    let Some(function) = node.child_by_field_name("function") else { return };
    let kind = match function.utf8_text(source.as_bytes()).ok() {
        Some("import") => ImportKind::Dynamic,
        Some("require") if function.kind() == "identifier" => ImportKind::Require,
        Some("require.resolve") if function.kind() == "member_expression" => ImportKind::Require,
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
const resolved = require.resolve('./resolved');
import legacyAlias = require('./legacy-alias');
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
                "./resolved",
                "./legacy-alias",
            ]
        );
        assert_eq!(imports[1].kind, ImportKind::TypeOnly);
        assert_eq!(imports[2].kind, ImportKind::ReExport);
        assert!(imports[7..].iter().all(|import| import.kind == ImportKind::Require));
    }

    #[test]
    fn extracts_directives_and_next_runtime_from_ast_semantics() {
        let parsed = JsTsParser
            .parse_module(
                &ModulePath("src/app/route.ts".into()),
                "/* header */\n'use server';\nexport const runtime = 'edge';\nconst later = 'use client';",
            )
            .unwrap();
        assert_eq!(parsed.semantics.directives, vec!["use server"]);
        assert_eq!(parsed.semantics.exported_runtime.as_deref(), Some("edge"));

        let nested = JsTsParser
            .parse_module(
                &ModulePath("src/not-runtime.ts".into()),
                "function f() { const runtime = 'nodejs'; }",
            )
            .unwrap();
        assert_eq!(nested.semantics.exported_runtime, None);

        for source in [
            "namespace Internal { export const runtime = 'edge'; }",
            "if (enabled) { const runtime = 'edge'; }",
            "export { runtime } from './runtime-config';",
        ] {
            let parsed =
                JsTsParser.parse_module(&ModulePath("src/app/page.ts".into()), source).unwrap();
            assert_eq!(parsed.semantics.exported_runtime, None, "source: {source}");
        }
        let escaped = JsTsParser
            .parse_module(
                &ModulePath("src/app/route.ts".into()),
                r#"export const runtime = "e\u0064ge";"#,
            )
            .unwrap();
        assert_eq!(escaped.semantics.exported_runtime.as_deref(), Some("edge"));
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
