use std::sync::LazyLock;

use regex::Regex;
use wae_core::domain::{
    Import, ImportKind, ModuleId, ModulePath, ParseError, ParseErrorKind, SourceLocation,
};

pub trait ParserAdapter: Send + Sync {
    fn parse_imports(
        &self,
        module_path: &ModulePath,
        source: &str,
    ) -> Result<Vec<Import>, ParseError>;
}

/// A dependency-oriented JS/TS syntax adapter. It deliberately extracts only module
/// declarations and call expressions required by the architecture IR.
#[derive(Debug, Default)]
pub struct JsTsParser;

static STATIC_IMPORT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?m)\bimport\s+(type\s+)?(?:[^;'\"\n]*?\s+from\s+)?['\"]([^'\"]+)['\"]"#)
        .expect("valid import regex")
});
static RE_EXPORT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?m)\bexport\s+(?:type\s+)?(?:\*|\{[^}]*\})\s+from\s+['\"]([^'\"]+)['\"]"#)
        .expect("valid export regex")
});
static DYNAMIC_IMPORT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"\bimport\s*\(\s*['\"]([^'\"]+)['\"]\s*\)"#).expect("valid dynamic import regex")
});
static REQUIRE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"\brequire\s*\(\s*['\"]([^'\"]+)['\"]\s*\)"#).expect("valid require regex")
});

impl ParserAdapter for JsTsParser {
    fn parse_imports(
        &self,
        module_path: &ModulePath,
        source: &str,
    ) -> Result<Vec<Import>, ParseError> {
        validate_syntax(module_path, source)?;
        let searchable = mask_comments(source)?;
        let mut imports = Vec::new();

        for captures in STATIC_IMPORT.captures_iter(&searchable) {
            let Some(specifier) = captures.get(2) else { continue };
            let kind =
                if captures.get(1).is_some() { ImportKind::TypeOnly } else { ImportKind::Static };
            imports.push(make_import(
                module_path,
                source,
                specifier.as_str(),
                specifier.start(),
                kind,
            ));
        }
        collect_calls(
            &mut imports,
            &DYNAMIC_IMPORT,
            module_path,
            source,
            &searchable,
            ImportKind::Dynamic,
        );
        collect_calls(
            &mut imports,
            &REQUIRE,
            module_path,
            source,
            &searchable,
            ImportKind::Require,
        );
        collect_calls(
            &mut imports,
            &RE_EXPORT,
            module_path,
            source,
            &searchable,
            ImportKind::ReExport,
        );

        imports.sort_by_key(|import| (import.location.line, import.location.column));
        imports.dedup_by(|a, b| {
            a.specifier == b.specifier && a.location == b.location && a.kind == b.kind
        });
        Ok(imports)
    }
}

fn validate_syntax(module_path: &ModulePath, source: &str) -> Result<(), ParseError> {
    let mut parser = tree_sitter::Parser::new();
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
    if tree.root_node().has_error() {
        let point = first_error(tree.root_node()).map(|node| node.start_position());
        return Err(ParseError {
            kind: ParseErrorKind::MalformedSource,
            message: "malformed JavaScript/TypeScript source".into(),
            location: point.map(|point| SourceLocation {
                file: module_path.0.clone(),
                line: point.row + 1,
                column: point.column + 1,
            }),
        });
    }
    Ok(())
}

fn first_error(node: tree_sitter::Node<'_>) -> Option<tree_sitter::Node<'_>> {
    if node.is_error() || node.is_missing() {
        return Some(node);
    }
    let mut cursor = node.walk();
    node.children(&mut cursor).find_map(first_error)
}

fn collect_calls(
    output: &mut Vec<Import>,
    regex: &Regex,
    path: &ModulePath,
    source: &str,
    searchable: &str,
    kind: ImportKind,
) {
    for captures in regex.captures_iter(searchable) {
        if let Some(specifier) = captures.get(1) {
            output.push(make_import(
                path,
                source,
                specifier.as_str(),
                specifier.start(),
                kind.clone(),
            ));
        }
    }
}

fn make_import(
    path: &ModulePath,
    source: &str,
    specifier: &str,
    byte_offset: usize,
    kind: ImportKind,
) -> Import {
    let (line, column) = line_column(source, byte_offset);
    Import {
        module_id: ModuleId(path.0.clone()),
        specifier: specifier.to_string(),
        kind,
        location: SourceLocation { file: path.0.clone(), line, column },
    }
}

fn line_column(source: &str, byte_offset: usize) -> (usize, usize) {
    let prefix = &source[..byte_offset.min(source.len())];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix
        .rsplit_once('\n')
        .map_or(prefix.chars().count() + 1, |(_, tail)| tail.chars().count() + 1);
    (line, column)
}

fn mask_comments(source: &str) -> Result<String, ParseError> {
    let bytes = source.as_bytes();
    let mut result = bytes.to_vec();
    let mut index = 0;
    let mut quote = None;
    while index < bytes.len() {
        if let Some(delimiter) = quote {
            if bytes[index] == b'\\' {
                index = (index + 2).min(bytes.len());
                continue;
            }
            if bytes[index] == delimiter {
                quote = None;
            }
            index += 1;
            continue;
        }
        if matches!(bytes[index], b'\'' | b'"' | b'`') {
            quote = Some(bytes[index]);
            index += 1;
            continue;
        }
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'/') {
            result[index] = b' ';
            result[index + 1] = b' ';
            index += 2;
            while index < bytes.len() && bytes[index] != b'\n' {
                result[index] = b' ';
                index += 1;
            }
            continue;
        }
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
            result[index] = b' ';
            result[index + 1] = b' ';
            index += 2;
            let mut closed = false;
            while index < bytes.len() {
                if bytes[index] == b'*' && bytes.get(index + 1) == Some(&b'/') {
                    result[index] = b' ';
                    result[index + 1] = b' ';
                    index += 2;
                    closed = true;
                    break;
                }
                if bytes[index] != b'\n' {
                    result[index] = b' ';
                }
                index += 1;
            }
            if !closed {
                return Err(parse_error("unterminated block comment"));
            }
            continue;
        }
        index += 1;
    }
    if quote.is_some() {
        return Err(parse_error("unterminated string literal"));
    }
    String::from_utf8(result).map_err(|_| parse_error("source is not valid UTF-8"))
}

fn parse_error(message: &str) -> ParseError {
    ParseError { kind: ParseErrorKind::MalformedSource, message: message.into(), location: None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_all_supported_dependency_forms_and_ignores_comments() {
        let source = r#"
import value from './static';
import type { T } from "./types";
export * from './reexport';
const lazy = import('./lazy');
const legacy = require('./legacy');
// import nope from './comment';
"#;
        let imports = JsTsParser.parse_imports(&ModulePath("src/a.ts".into()), source).unwrap();
        assert_eq!(
            imports.iter().map(|i| i.specifier.as_str()).collect::<Vec<_>>(),
            vec!["./static", "./types", "./reexport", "./lazy", "./legacy"]
        );
        assert_eq!(imports[1].kind, ImportKind::TypeOnly);
        assert_eq!(imports[2].kind, ImportKind::ReExport);
    }

    #[test]
    fn reports_malformed_typescript_from_the_syntax_tree() {
        let error = JsTsParser
            .parse_imports(&ModulePath("src/broken.ts".into()), "export const value = ;")
            .unwrap_err();
        assert_eq!(error.kind, ParseErrorKind::MalformedSource);
        assert_eq!(error.location.unwrap().line, 1);
    }
}
