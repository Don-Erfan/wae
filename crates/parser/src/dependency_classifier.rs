use tree_sitter::Node;
use wae_core::domain::ImportKind;

pub(super) fn classify_import(statement: Node<'_>, source: &str) -> ImportKind {
    if has_direct_type_keyword(statement) {
        return ImportKind::TypeOnly;
    }
    let Some(clause) = direct_named_child(statement, "import_clause") else {
        return ImportKind::Static;
    };
    let children = named_children(clause);
    if children.len() == 1
        && children[0].kind() == "named_imports"
        && all_specifiers_are_type(children[0], "import_specifier", source)
    {
        ImportKind::TypeOnly
    } else {
        ImportKind::Static
    }
}

pub(super) fn classify_export(statement: Node<'_>, source: &str) -> ImportKind {
    if has_direct_type_keyword(statement) {
        return ImportKind::TypeOnly;
    }
    direct_named_child(statement, "export_clause")
        .filter(|clause| all_specifiers_are_type(*clause, "export_specifier", source))
        .map_or(ImportKind::ReExport, |_| ImportKind::TypeOnly)
}

fn all_specifiers_are_type(container: Node<'_>, kind: &str, source: &str) -> bool {
    let specifiers = named_children(container)
        .into_iter()
        .filter(|child| child.kind() == kind)
        .collect::<Vec<_>>();
    !specifiers.is_empty()
        && specifiers.iter().all(|specifier| {
            has_direct_type_keyword(*specifier)
                || specifier.utf8_text(source.as_bytes()).ok().is_some_and(starts_with_type_keyword)
        })
}

fn starts_with_type_keyword(value: &str) -> bool {
    value
        .trim_start()
        .strip_prefix("type")
        .is_some_and(|rest| rest.chars().next().is_some_and(|character| character.is_whitespace()))
}

fn has_direct_type_keyword(node: Node<'_>) -> bool {
    (0..node.child_count()).any(|index| {
        node.child(index).is_some_and(|child| !child.is_named() && child.kind() == "type")
    })
}

fn direct_named_child<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    named_children(node).into_iter().find(|child| child.kind() == kind)
}

fn named_children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}
