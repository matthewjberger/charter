use anyhow::{Result, anyhow};
use std::cell::RefCell;
use tree_sitter::{Node, Parser, Tree};

use crate::extract::attributes::{CfgInfo, DeriveInfo};
use crate::extract::imports::{ImportInfo, ReExport};
use crate::extract::symbols::{
    AssociatedType, EnumVariant, FileSymbols, ImplMethod, InherentImpl, MacroInfo, StructField,
    Symbol, SymbolKind, TraitMethod, VariantPayload, Visibility,
};

thread_local! {
    static PARSER: RefCell<Parser> = RefCell::new({
        let mut parser = Parser::new();
        parser.set_language(&tree_sitter_rust::LANGUAGE.into()).expect("Rust grammar");
        parser.set_timeout_micros(10_000_000);
        parser
    });
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ParsedFile {
    pub symbols: FileSymbols,
    pub module_doc: Option<String>,
    pub derives: Vec<DeriveInfo>,
    pub cfgs: Vec<CfgInfo>,
    pub imports: Vec<ImportInfo>,
    pub re_exports: Vec<ReExport>,
    pub has_test_module: bool,
    pub test_functions: Vec<String>,
    pub identifier_locations: Vec<(String, usize)>,
}

pub fn parse_rust_file(content: &str) -> Result<ParsedFile> {
    PARSER.with(|parser| {
        let mut parser = parser.borrow_mut();
        let tree = parser
            .parse(content, None)
            .ok_or_else(|| anyhow!("Failed to parse file"))?;

        extract_from_tree(&tree, content)
    })
}

fn extract_from_tree(tree: &Tree, source: &str) -> Result<ParsedFile> {
    let root = tree.root_node();
    let source_bytes = source.as_bytes();

    let mut result = ParsedFile::default();

    extract_module_doc(&root, source_bytes, &mut result);
    extract_items(&root, source_bytes, &mut result);
    extract_identifier_locations(&root, source_bytes, &mut result);

    Ok(result)
}

fn extract_module_doc(root: &Node, source: &[u8], result: &mut ParsedFile) {
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        if child.kind() == "line_comment" {
            let text = node_text(&child, source);
            if text.starts_with("//!") {
                let doc = text.strip_prefix("//!").unwrap_or("").trim();
                if result.module_doc.is_none() {
                    result.module_doc = Some(doc.to_string());
                } else if let Some(existing) = &mut result.module_doc {
                    existing.push(' ');
                    existing.push_str(doc);
                }
            }
        } else if child.kind() == "block_comment" {
            let text = node_text(&child, source);
            if text.starts_with("/*!") {
                let doc = text
                    .strip_prefix("/*!")
                    .and_then(|s| s.strip_suffix("*/"))
                    .unwrap_or("")
                    .trim();
                result.module_doc = Some(doc.to_string());
            }
        } else if child.kind() != "line_comment" && child.kind() != "block_comment" {
            break;
        }
    }
}

fn extract_items(node: &Node, source: &[u8], result: &mut ParsedFile) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "struct_item" => extract_struct(&child, source, result),
            "enum_item" => extract_enum(&child, source, result),
            "trait_item" => extract_trait(&child, source, result),
            "impl_item" => extract_impl(&child, source, result),
            "function_item" => extract_function(&child, source, result),
            "const_item" => extract_const(&child, source, result),
            "static_item" => extract_static(&child, source, result),
            "type_item" => extract_type_alias(&child, source, result),
            "mod_item" => extract_mod(&child, source, result),
            "use_declaration" => extract_use(&child, source, result),
            "attribute_item" => extract_attribute(&child, source, result),
            "macro_definition" => extract_macro(&child, source, result),
            _ => {}
        }
    }
}

fn extract_struct(node: &Node, source: &[u8], result: &mut ParsedFile) {
    let visibility = extract_visibility(node, source);
    let name = find_child_text(node, "type_identifier", source).unwrap_or_default();
    let generics = extract_generics(node, source);
    let line = node.start_position().row + 1;

    let mut fields = Vec::new();

    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            if child.kind() == "field_declaration" {
                let field_vis = extract_visibility(&child, source);
                let field_name =
                    find_child_text(&child, "field_identifier", source).unwrap_or_default();
                let field_type = child
                    .child_by_field_name("type")
                    .map(|n| node_text(&n, source))
                    .unwrap_or_default();

                fields.push(StructField {
                    name: field_name,
                    field_type,
                    visibility: field_vis,
                });
            }
        }
    }

    let derives = extract_derives_for_item(node, source);
    for derive in &derives {
        result.derives.push(DeriveInfo {
            target: name.clone(),
            traits: derive.clone(),
            line,
        });
    }

    result.symbols.symbols.push(Symbol {
        name,
        kind: SymbolKind::Struct { fields },
        visibility,
        generics,
        line,
        is_async: false,
        is_unsafe: false,
        is_const: false,
        re_exported_as: None,
    });
}

fn extract_enum(node: &Node, source: &[u8], result: &mut ParsedFile) {
    let visibility = extract_visibility(node, source);
    let name = find_child_text(node, "type_identifier", source).unwrap_or_default();
    let generics = extract_generics(node, source);
    let line = node.start_position().row + 1;

    let mut variants = Vec::new();

    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            if child.kind() == "enum_variant" {
                let variant_name =
                    find_child_text(&child, "identifier", source).unwrap_or_default();

                let payload = if let Some(tuple_body) = child
                    .children(&mut child.walk())
                    .find(|n| n.kind() == "ordered_field_declaration_list")
                {
                    let mut fields = Vec::new();
                    let mut tuple_cursor = tuple_body.walk();
                    for field in tuple_body.children(&mut tuple_cursor) {
                        if field.kind() == "ordered_field_declaration" {
                            if let Some(type_node) = field.child_by_field_name("type") {
                                fields.push(node_text(&type_node, source));
                            }
                        }
                    }
                    if fields.is_empty() {
                        for field in tuple_body.children(&mut tuple_cursor) {
                            if field.kind() == "type_identifier"
                                || field.kind() == "generic_type"
                                || field.kind() == "reference_type"
                                || field.kind() == "primitive_type"
                            {
                                fields.push(node_text(&field, source));
                            }
                        }
                    }
                    if !fields.is_empty() {
                        Some(VariantPayload::Tuple(fields))
                    } else {
                        None
                    }
                } else if let Some(struct_body) = child
                    .children(&mut child.walk())
                    .find(|n| n.kind() == "field_declaration_list")
                {
                    let mut fields = Vec::new();
                    let mut struct_cursor = struct_body.walk();
                    for field in struct_body.children(&mut struct_cursor) {
                        if field.kind() == "field_declaration" {
                            let field_name = find_child_text(&field, "field_identifier", source)
                                .unwrap_or_default();
                            let field_type = field
                                .child_by_field_name("type")
                                .map(|n| node_text(&n, source))
                                .unwrap_or_default();
                            fields.push((field_name, field_type));
                        }
                    }
                    if !fields.is_empty() {
                        Some(VariantPayload::Struct(fields))
                    } else {
                        None
                    }
                } else {
                    None
                };

                variants.push(EnumVariant {
                    name: variant_name,
                    payload,
                });
            }
        }
    }

    let derives = extract_derives_for_item(node, source);
    for derive in &derives {
        result.derives.push(DeriveInfo {
            target: name.clone(),
            traits: derive.clone(),
            line,
        });
    }

    result.symbols.symbols.push(Symbol {
        name,
        kind: SymbolKind::Enum { variants },
        visibility,
        generics,
        line,
        is_async: false,
        is_unsafe: false,
        is_const: false,
        re_exported_as: None,
    });
}

fn extract_trait(node: &Node, source: &[u8], result: &mut ParsedFile) {
    let visibility = extract_visibility(node, source);
    let name = find_child_text(node, "type_identifier", source).unwrap_or_default();
    let generics = extract_generics(node, source);
    let line = node.start_position().row + 1;

    let mut supertraits = Vec::new();
    let mut methods = Vec::new();
    let mut associated_types = Vec::new();

    if let Some(bounds) = node.child_by_field_name("bounds") {
        let bounds_text = node_text(&bounds, source);
        for bound in bounds_text.split('+') {
            let bound = bound.trim();
            if !bound.is_empty() {
                supertraits.push(bound.to_string());
            }
        }
    }

    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            match child.kind() {
                "function_signature_item" => {
                    let method_name =
                        find_child_text(&child, "identifier", source).unwrap_or_default();
                    let signature = extract_function_signature(&child, source);
                    methods.push(TraitMethod {
                        name: method_name,
                        signature,
                        has_default: false,
                    });
                }
                "function_item" => {
                    let method_name =
                        find_child_text(&child, "identifier", source).unwrap_or_default();
                    let signature = extract_function_signature(&child, source);
                    methods.push(TraitMethod {
                        name: method_name,
                        signature,
                        has_default: true,
                    });
                }
                "associated_type" => {
                    let type_name =
                        find_child_text(&child, "type_identifier", source).unwrap_or_default();
                    let bounds = child
                        .child_by_field_name("bounds")
                        .map(|n| node_text(&n, source));
                    associated_types.push(AssociatedType {
                        name: type_name,
                        bounds,
                    });
                }
                _ => {}
            }
        }
    }

    result.symbols.symbols.push(Symbol {
        name,
        kind: SymbolKind::Trait {
            supertraits,
            methods,
            associated_types,
        },
        visibility,
        generics,
        line,
        is_async: false,
        is_unsafe: false,
        is_const: false,
        re_exported_as: None,
    });
}

fn extract_impl(node: &Node, source: &[u8], result: &mut ParsedFile) {
    let trait_name = node
        .child_by_field_name("trait")
        .map(|n| node_text(&n, source));

    let type_node = node.child_by_field_name("type");
    let type_name = type_node.map(|n| node_text(&n, source)).unwrap_or_default();

    let base_type_name = extract_base_type_name(&type_name);

    let impl_generics = extract_type_parameters(node, source);
    let where_clause = extract_where_clause(node, source);

    let mut methods = Vec::new();

    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            if child.kind() == "function_item" {
                let visibility = extract_visibility(&child, source);
                let fn_name = find_child_text(&child, "identifier", source).unwrap_or_default();
                let signature = extract_function_signature(&child, source);
                let is_async = has_modifier(&child, "async");
                let is_unsafe = has_modifier(&child, "unsafe");
                let is_const = has_modifier(&child, "const");
                let fn_line = child.start_position().row + 1;

                methods.push(ImplMethod {
                    name: fn_name,
                    visibility,
                    signature,
                    is_async,
                    is_unsafe,
                    is_const,
                    line: fn_line,
                });
            }
        }
    }

    if let Some(trait_name) = trait_name {
        result.symbols.impl_map.push((trait_name, type_name));
    } else if !methods.is_empty() {
        result.symbols.inherent_impls.push(InherentImpl {
            type_name: base_type_name,
            generics: impl_generics,
            where_clause,
            methods,
        });
    }
}

fn extract_base_type_name(full_type: &str) -> String {
    let trimmed = full_type.trim();
    if let Some(angle_pos) = trimmed.find('<') {
        trimmed[..angle_pos].trim().to_string()
    } else {
        trimmed.to_string()
    }
}

fn extract_type_parameters(node: &Node, source: &[u8]) -> String {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_parameters" {
            return node_text(&child, source);
        }
    }
    String::new()
}

fn extract_where_clause(node: &Node, source: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "where_clause" {
            let text = node_text(&child, source);
            let text = text.strip_prefix("where").unwrap_or(&text).trim();
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
    }
    None
}

fn extract_function(node: &Node, source: &[u8], result: &mut ParsedFile) {
    let visibility = extract_visibility(node, source);
    let name = find_child_text(node, "identifier", source).unwrap_or_default();
    let generics = extract_generics(node, source);
    let signature = extract_function_signature(node, source);
    let line = node.start_position().row + 1;

    let is_async = has_modifier(node, "async");
    let is_unsafe = has_modifier(node, "unsafe");
    let is_const = has_modifier(node, "const");

    if has_test_attribute(node, source) && !name.is_empty() {
        result.test_functions.push(name.clone());
    }

    result.symbols.symbols.push(Symbol {
        name,
        kind: SymbolKind::Function { signature },
        visibility,
        generics,
        line,
        is_async,
        is_unsafe,
        is_const,
        re_exported_as: None,
    });
}

fn has_test_attribute(node: &Node, source: &[u8]) -> bool {
    if let Some(parent) = node.parent() {
        let mut cursor = parent.walk();
        for sibling in parent.children(&mut cursor) {
            if sibling.end_byte() < node.start_byte() && sibling.kind() == "attribute_item" {
                let text = node_text(&sibling, source);
                if text.contains("#[test]") || text.contains("#[tokio::test") || text.contains("#[async_std::test") {
                    return true;
                }
            }
        }
    }
    false
}

fn extract_const(node: &Node, source: &[u8], result: &mut ParsedFile) {
    let visibility = extract_visibility(node, source);
    let name = find_child_text(node, "identifier", source).unwrap_or_default();
    let const_type = node
        .child_by_field_name("type")
        .map(|n| node_text(&n, source))
        .unwrap_or_default();
    let line = node.start_position().row + 1;

    let value = node
        .child_by_field_name("value")
        .and_then(|n| extract_simple_value(&n, source));

    result.symbols.symbols.push(Symbol {
        name,
        kind: SymbolKind::Const { const_type, value },
        visibility,
        generics: String::new(),
        line,
        is_async: false,
        is_unsafe: false,
        is_const: true,
        re_exported_as: None,
    });
}

fn extract_static(node: &Node, source: &[u8], result: &mut ParsedFile) {
    let visibility = extract_visibility(node, source);
    let name = find_child_text(node, "identifier", source).unwrap_or_default();
    let static_type = node
        .child_by_field_name("type")
        .map(|n| node_text(&n, source))
        .unwrap_or_default();
    let line = node.start_position().row + 1;

    let is_mutable = node
        .children(&mut node.walk())
        .any(|c| c.kind() == "mutable_specifier");

    let value = node
        .child_by_field_name("value")
        .and_then(|n| extract_simple_value(&n, source));

    result.symbols.symbols.push(Symbol {
        name,
        kind: SymbolKind::Static {
            static_type,
            is_mutable,
            value,
        },
        visibility,
        generics: String::new(),
        line,
        is_async: false,
        is_unsafe: false,
        is_const: false,
        re_exported_as: None,
    });
}

fn extract_simple_value(node: &Node, source: &[u8]) -> Option<String> {
    let text = node_text(node, source);
    let trimmed = text.trim();

    if trimmed.contains('\n') || trimmed.len() > 80 {
        return None;
    }

    match node.kind() {
        "integer_literal" | "float_literal" | "string_literal" | "char_literal"
        | "boolean_literal" | "raw_string_literal" => Some(trimmed.to_string()),
        "unary_expression" | "binary_expression" => {
            if trimmed.len() <= 40 {
                Some(trimmed.to_string())
            } else {
                None
            }
        }
        "call_expression" | "struct_expression" => {
            if trimmed.len() <= 80 {
                Some(trimmed.to_string())
            } else {
                None
            }
        }
        "array_expression" => {
            if trimmed.len() <= 60 {
                Some(trimmed.to_string())
            } else {
                None
            }
        }
        "identifier" | "scoped_identifier" => Some(trimmed.to_string()),
        _ => {
            if trimmed.len() <= 50 && !trimmed.contains("||") && !trimmed.contains("&&") {
                Some(trimmed.to_string())
            } else {
                None
            }
        }
    }
}

fn extract_type_alias(node: &Node, source: &[u8], result: &mut ParsedFile) {
    let visibility = extract_visibility(node, source);
    let name = find_child_text(node, "type_identifier", source).unwrap_or_default();
    let generics = extract_generics(node, source);
    let aliased_type = node
        .child_by_field_name("type")
        .map(|n| node_text(&n, source))
        .unwrap_or_default();
    let line = node.start_position().row + 1;

    result.symbols.symbols.push(Symbol {
        name,
        kind: SymbolKind::TypeAlias { aliased_type },
        visibility,
        generics,
        line,
        is_async: false,
        is_unsafe: false,
        is_const: false,
        re_exported_as: None,
    });
}

fn extract_mod(node: &Node, source: &[u8], result: &mut ParsedFile) {
    let visibility = extract_visibility(node, source);
    let name = find_child_text(node, "identifier", source).unwrap_or_default();
    let line = node.start_position().row + 1;

    let mut cursor = node.walk();
    let has_cfg_test = node.children(&mut cursor).any(|child| {
        if child.kind() == "attribute_item" {
            let text = node_text(&child, source);
            text.contains("cfg(test)")
        } else {
            false
        }
    });

    if has_cfg_test {
        result.has_test_module = true;
    }

    if node.child_by_field_name("body").is_none() {
        result.symbols.symbols.push(Symbol {
            name,
            kind: SymbolKind::Mod,
            visibility,
            generics: String::new(),
            line,
            is_async: false,
            is_unsafe: false,
            is_const: false,
            re_exported_as: None,
        });
    }
}

fn extract_macro(node: &Node, source: &[u8], result: &mut ParsedFile) {
    let name = find_child_text(node, "identifier", source).unwrap_or_default();
    let line = node.start_position().row + 1;

    let is_exported = if let Some(parent) = node.parent() {
        let mut cursor = parent.walk();
        parent.children(&mut cursor).any(|sibling| {
            if sibling.end_byte() < node.start_byte() && sibling.kind() == "attribute_item" {
                let text = node_text(&sibling, source);
                text.contains("macro_export")
            } else {
                false
            }
        })
    } else {
        false
    };

    result.symbols.macros.push(MacroInfo {
        name,
        is_exported,
        line,
    });
}

fn extract_use(node: &Node, source: &[u8], result: &mut ParsedFile) {
    let visibility = extract_visibility(node, source);
    let line = node.start_position().row + 1;

    if let Some(arg) = node.child_by_field_name("argument") {
        let path = node_text(&arg, source);

        if visibility != Visibility::Private {
            result.re_exports.push(ReExport {
                source_path: path.clone(),
                visibility: visibility.clone(),
                line,
            });
        }

        result.imports.push(ImportInfo { path, line });
    }
}

fn extract_attribute(node: &Node, source: &[u8], result: &mut ParsedFile) {
    let text = node_text(node, source);
    let line = node.start_position().row + 1;

    if text.contains("#[cfg(") || text.contains("#[cfg_attr(") {
        let cfg_content = extract_cfg_content(&text);
        if let Some(condition) = cfg_content {
            result.cfgs.push(CfgInfo { condition, line });
        }
    }
}

fn extract_derives_for_item(node: &Node, source: &[u8]) -> Vec<Vec<String>> {
    let mut derives = Vec::new();
    let mut cursor = node.walk();

    let parent = node.parent();
    if let Some(parent) = parent {
        let mut sibling_cursor = parent.walk();
        for sibling in parent.children(&mut sibling_cursor) {
            if sibling.end_byte() >= node.start_byte() {
                break;
            }
            if sibling.kind() == "attribute_item" {
                let text = node_text(&sibling, source);
                if text.contains("#[derive(") {
                    if let Some(traits) = extract_derive_traits(&text) {
                        derives.push(traits);
                    }
                }
            }
        }
    }

    for child in node.children(&mut cursor) {
        if child.kind() == "attribute_item" {
            let text = node_text(&child, source);
            if text.contains("#[derive(") {
                if let Some(traits) = extract_derive_traits(&text) {
                    derives.push(traits);
                }
            }
        }
    }

    derives
}

fn extract_derive_traits(attr_text: &str) -> Option<Vec<String>> {
    let start = attr_text.find("#[derive(")? + 9;
    let end = attr_text[start..].find(')')? + start;
    let content = &attr_text[start..end];

    let traits: Vec<String> = content
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    if traits.is_empty() {
        None
    } else {
        Some(traits)
    }
}

fn extract_cfg_content(attr_text: &str) -> Option<String> {
    if let Some(start) = attr_text.find("#[cfg(") {
        let start = start + 6;
        let mut depth = 1;
        let mut end = start;
        for (index, char) in attr_text[start..].char_indices() {
            match char {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = start + index;
                        break;
                    }
                }
                _ => {}
            }
        }
        return Some(attr_text[start..end].to_string());
    }
    None
}

fn extract_visibility(node: &Node, source: &[u8]) -> Visibility {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "visibility_modifier" {
            let text = node_text(&child, source);
            return match text.as_str() {
                "pub" => Visibility::Public,
                _ if text.starts_with("pub(crate)") => Visibility::PubCrate,
                _ if text.starts_with("pub(super)") => Visibility::PubSuper,
                _ if text.starts_with("pub(self)") => Visibility::Private,
                _ if text.starts_with("pub(in") => Visibility::PubIn(text),
                _ => Visibility::Public,
            };
        }
    }
    Visibility::Private
}

fn extract_generics(node: &Node, source: &[u8]) -> String {
    if let Some(type_params) = node.child_by_field_name("type_parameters") {
        return node_text(&type_params, source);
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_parameters" {
            return node_text(&child, source);
        }
    }

    String::new()
}

fn extract_function_signature(node: &Node, source: &[u8]) -> String {
    let mut parts = Vec::new();

    if let Some(params) = node.child_by_field_name("parameters") {
        parts.push(node_text(&params, source));
    }

    if let Some(return_type) = node.child_by_field_name("return_type") {
        let ret = node_text(&return_type, source);
        parts.push(format!(" -> {}", ret.trim_start_matches("->")));
    }

    parts.join("")
}

fn has_modifier(node: &Node, modifier: &str) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == modifier {
            return true;
        }
    }
    false
}

fn find_child_text(node: &Node, kind: &str, source: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return Some(node_text(&child, source));
        }
        if child.kind() == "name" {
            if let Some(name_child) = child.child(0) {
                if name_child.kind() == kind {
                    return Some(node_text(&name_child, source));
                }
            }
            return Some(node_text(&child, source));
        }
    }
    None
}

fn node_text(node: &Node, source: &[u8]) -> String {
    node.utf8_text(source).unwrap_or("").to_string()
}

fn extract_identifier_locations(root: &Node, source: &[u8], result: &mut ParsedFile) {
    collect_identifiers(root, source, &mut result.identifier_locations);
}

fn collect_identifiers(node: &Node, source: &[u8], locations: &mut Vec<(String, usize)>) {
    if node.kind() == "type_identifier" || node.kind() == "identifier" {
        let name = node_text(node, source);
        if super::is_pascal_case(&name) {
            let line = node.start_position().row + 1;
            locations.push((name, line));
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_identifiers(&child, source, locations);
    }
}
