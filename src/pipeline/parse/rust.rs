use anyhow::{Result, anyhow};
use std::cell::RefCell;
use tree_sitter::{Node, Parser, Tree};

use crate::extract::attributes::{CfgInfo, DeriveInfo};
use crate::extract::calls::{CallEdge, CallInfo};
use crate::extract::complexity::{ComplexityMetrics, FunctionComplexity};
use crate::extract::errors::{
    ErrorInfo, ErrorOrigin, ErrorOriginKind, ErrorReturnType, PropagationPoint,
};
use crate::extract::imports::{ImportInfo, ReExport};
use crate::extract::safety::{
    AsyncFunction, AsyncInfo, AwaitPoint, BlockingCall, BorrowInfo, CfgBlock, ComplexBound,
    FeatureGate, FunctionLifetime, GatedSymbol, ItemConstraints, ItemDoc, LifetimeInfo, PanicKind,
    PanicPoint, SafetyInfo, SpawnPoint, SpawnType, StructLifetime, TestFunction, TestInfo,
    TestModule, TestedItem, TypeParam, UnsafeBlock, UnsafeImpl, UnsafeOperation,
};
use crate::extract::symbols::{
    AssociatedType, BodySummary, EnumVariant, FunctionBody, ImplMethod, InherentImpl, MacroInfo,
    StructField, Symbol, SymbolKind, TraitMethod, VariantPayload, Visibility,
};

use super::{CapturedBody, ParsedFile};

thread_local! {
    static PARSER: RefCell<Parser> = RefCell::new({
        let mut parser = Parser::new();
        parser.set_language(&tree_sitter_rust::LANGUAGE.into()).expect("Rust grammar");
        parser.set_timeout_micros(10_000_000);
        parser
    });
}

pub fn parse_rust_file(content: &str, file_path: &str) -> Result<ParsedFile> {
    PARSER.with(|parser| {
        let mut parser = parser.borrow_mut();
        let tree = parser
            .parse(content, None)
            .ok_or_else(|| anyhow!("Failed to parse file"))?;

        extract_from_tree(&tree, content, file_path)
    })
}

fn extract_from_tree(tree: &Tree, source: &str, file_path: &str) -> Result<ParsedFile> {
    let root = tree.root_node();
    let source_bytes = source.as_bytes();

    let mut result = ParsedFile::default();

    extract_module_doc(&root, source_bytes, &mut result);
    extract_items(&root, source_bytes, &mut result);
    extract_identifier_locations(&root, source_bytes, &mut result);
    extract_phase1_data(&root, source_bytes, file_path, &mut result);
    extract_safety_info(&root, source_bytes, &mut result);
    extract_lifetime_info(&root, source_bytes, &mut result);
    extract_async_info(&root, source_bytes, &mut result);
    extract_feature_flags(&root, source_bytes, &mut result);
    extract_doc_comments(&root, source_bytes, &mut result);
    extract_generic_constraints(&root, source_bytes, &mut result);
    extract_test_info(&root, source_bytes, &mut result);

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
                    body: None,
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
        kind: SymbolKind::Function {
            signature,
            body: None,
        },
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
                if text.contains("#[test]")
                    || text.contains("#[tokio::test")
                    || text.contains("#[async_std::test")
                {
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

        result.imports.push(ImportInfo {
            path,
            line,
            kind: crate::extract::imports::ImportKind::RustUse,
        });
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

fn extract_phase1_data(root: &Node, source: &[u8], file_path: &str, result: &mut ParsedFile) {
    let mut cursor = root.walk();
    let mut snippet_budget = MAX_TOTAL_SNIPPET_BUDGET / 20;

    for child in root.children(&mut cursor) {
        match child.kind() {
            "function_item" => {
                extract_function_phase1(
                    &child,
                    source,
                    file_path,
                    None,
                    result,
                    &mut snippet_budget,
                );
            }
            "impl_item" => {
                extract_impl_phase1(&child, source, file_path, result, &mut snippet_budget);
            }
            _ => {}
        }
    }
}

fn extract_impl_phase1(
    node: &Node,
    source: &[u8],
    file_path: &str,
    result: &mut ParsedFile,
    snippet_budget: &mut usize,
) {
    let type_node = node.child_by_field_name("type");
    let type_name = type_node.map(|n| node_text(&n, source));
    let base_type = type_name.as_ref().map(|t| extract_base_type_name(t));

    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            if child.kind() == "function_item" {
                extract_function_phase1(
                    &child,
                    source,
                    file_path,
                    base_type.clone(),
                    result,
                    snippet_budget,
                );
            }
        }
    }
}

fn extract_function_phase1(
    node: &Node,
    source: &[u8],
    file_path: &str,
    impl_type: Option<String>,
    result: &mut ParsedFile,
    snippet_budget: &mut usize,
) {
    let name = find_child_text(node, "identifier", source).unwrap_or_default();
    if name.is_empty() {
        return;
    }

    let line = node.start_position().row + 1;
    let visibility = extract_visibility(node, source);
    let is_public = matches!(visibility, Visibility::Public | Visibility::PubCrate);
    let is_test = has_test_attribute(node, source);

    let body = node.child_by_field_name("body");

    let (cyclomatic, nesting_depth, line_count) = if let Some(ref body) = body {
        (
            compute_cyclomatic_complexity(body, source),
            compute_nesting_depth(body),
            compute_line_count(body),
        )
    } else {
        (1, 0, 0)
    };

    let metrics = ComplexityMetrics {
        cyclomatic,
        line_count,
        nesting_depth,
        call_sites: 0,
        churn_score: 0,
        is_public,
        is_test,
    };

    let importance_score = metrics.importance_score();

    result.complexity.push(FunctionComplexity {
        name: name.clone(),
        impl_type: impl_type.clone(),
        line,
        metrics,
    });

    if let Some(ref body_node) = body {
        if let Some(captured_body) =
            capture_function_body(body_node, source, importance_score, snippet_budget)
        {
            result.captured_bodies.push(CapturedBody {
                function_name: name.clone(),
                impl_type: impl_type.clone(),
                line,
                body: captured_body,
                importance_score,
            });
        }
    }

    let mut call_info = CallInfo::new(file_path.to_string(), name.clone(), impl_type.clone(), line);

    if let Some(ref body) = body {
        extract_calls_from_body(body, source, &mut call_info.callees);
    }

    if !call_info.callees.is_empty() {
        result.call_graph.push(call_info);
    }

    let return_type = extract_error_return_type(node, source);
    if return_type.is_fallible() {
        let mut error_info =
            ErrorInfo::new(file_path.to_string(), name, impl_type, return_type, line);

        if let Some(ref body) = body {
            extract_error_propagation(body, source, &mut error_info);
            extract_error_origins(body, source, &mut error_info);
        }

        result.error_info.push(error_info);
    }
}

fn compute_cyclomatic_complexity(node: &Node, source: &[u8]) -> u32 {
    let mut complexity = 1;
    count_branch_points(node, source, &mut complexity);
    complexity
}

fn count_branch_points(node: &Node, source: &[u8], complexity: &mut u32) {
    match node.kind() {
        "if_expression" | "while_expression" | "for_expression" | "loop_expression" => {
            *complexity += 1;
        }
        "match_expression" => {
            if let Some(body) = node.child_by_field_name("body") {
                let mut cursor = body.walk();
                let arm_count = body
                    .children(&mut cursor)
                    .filter(|c| c.kind() == "match_arm")
                    .count();
                *complexity += arm_count.saturating_sub(1) as u32;
            }
        }
        "try_expression" => {
            *complexity += 1;
        }
        "binary_expression" => {
            let text = node_text(node, source);
            if text.contains("&&") || text.contains("||") {
                *complexity += 1;
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        count_branch_points(&child, source, complexity);
    }
}

fn compute_nesting_depth(node: &Node) -> u32 {
    let mut max_depth = 0;
    compute_nesting_depth_recursive(node, 0, &mut max_depth);
    max_depth
}

fn compute_nesting_depth_recursive(node: &Node, current_depth: u32, max_depth: &mut u32) {
    let is_nesting = matches!(
        node.kind(),
        "if_expression"
            | "while_expression"
            | "for_expression"
            | "loop_expression"
            | "match_expression"
            | "closure_expression"
    );

    let new_depth = if is_nesting {
        current_depth + 1
    } else {
        current_depth
    };

    if new_depth > *max_depth {
        *max_depth = new_depth;
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        compute_nesting_depth_recursive(&child, new_depth, max_depth);
    }
}

fn compute_line_count(node: &Node) -> u32 {
    let start_line = node.start_position().row;
    let end_line = node.end_position().row;
    (end_line - start_line + 1) as u32
}

fn extract_calls_from_body(node: &Node, source: &[u8], callees: &mut Vec<CallEdge>) {
    match node.kind() {
        "call_expression" => {
            if let Some(function) = node.child_by_field_name("function") {
                let line = node.start_position().row + 1;
                let is_try = is_inside_try(node);
                let is_async = is_await_call(node, source);

                match function.kind() {
                    "identifier" => {
                        let name = node_text(&function, source);
                        callees.push(CallEdge {
                            target: name,
                            target_type: None,
                            line,
                            is_async_call: is_async,
                            is_try_call: is_try,
                        });
                    }
                    "field_expression" => {
                        if let Some(field) = function.child_by_field_name("field") {
                            let method_name = node_text(&field, source);
                            let receiver_type = function
                                .child_by_field_name("value")
                                .map(|v| infer_receiver_type(&v, source));
                            callees.push(CallEdge {
                                target: method_name,
                                target_type: receiver_type,
                                line,
                                is_async_call: is_async,
                                is_try_call: is_try,
                            });
                        }
                    }
                    "scoped_identifier" => {
                        let full_path = node_text(&function, source);
                        let parts: Vec<&str> = full_path.split("::").collect();
                        if parts.len() >= 2 {
                            let type_name = parts[..parts.len() - 1].join("::");
                            let method_name = parts[parts.len() - 1].to_string();
                            callees.push(CallEdge {
                                target: method_name,
                                target_type: Some(type_name),
                                line,
                                is_async_call: is_async,
                                is_try_call: is_try,
                            });
                        } else {
                            callees.push(CallEdge {
                                target: full_path,
                                target_type: None,
                                line,
                                is_async_call: is_async,
                                is_try_call: is_try,
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
        "macro_invocation" => {
            if let Some(macro_node) = node.child_by_field_name("macro") {
                let macro_name = node_text(&macro_node, source);
                let line = node.start_position().row + 1;
                callees.push(CallEdge {
                    target: format!("{}!", macro_name),
                    target_type: None,
                    line,
                    is_async_call: false,
                    is_try_call: false,
                });
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        extract_calls_from_body(&child, source, callees);
    }
}

fn is_inside_try(node: &Node) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "try_expression" {
            return true;
        }
        current = parent.parent();
    }
    false
}

fn is_await_call(node: &Node, source: &[u8]) -> bool {
    if let Some(parent) = node.parent() {
        if parent.kind() == "await_expression" {
            return true;
        }
    }
    let text = node_text(node, source);
    text.contains(".await")
}

fn infer_receiver_type(node: &Node, source: &[u8]) -> String {
    match node.kind() {
        "identifier" => {
            let name = node_text(node, source);
            if name == "self" {
                "Self".to_string()
            } else {
                name
            }
        }
        "call_expression" => {
            if let Some(function) = node.child_by_field_name("function") {
                let text = node_text(&function, source);
                if let Some(type_name) = text.split("::").next() {
                    return type_name.to_string();
                }
            }
            "?".to_string()
        }
        _ => "?".to_string(),
    }
}

fn extract_error_return_type(node: &Node, source: &[u8]) -> ErrorReturnType {
    let return_type = node.child_by_field_name("return_type");
    let return_type = match return_type {
        Some(rt) => rt,
        None => return ErrorReturnType::Neither,
    };

    let type_text = node_text(&return_type, source);
    let type_text = type_text.trim_start_matches("->");
    let type_text = type_text.trim();

    if type_text.starts_with("Result<") || type_text.starts_with("anyhow::Result<") {
        let inner = extract_generic_params(type_text);
        let parts: Vec<&str> = inner.splitn(2, ',').collect();
        let ok_type = parts
            .first()
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        let err_type = parts
            .get(1)
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "Error".to_string());
        ErrorReturnType::Result { ok_type, err_type }
    } else if type_text.starts_with("Option<") {
        let inner = extract_generic_params(type_text);
        ErrorReturnType::Option { some_type: inner }
    } else {
        ErrorReturnType::Neither
    }
}

fn extract_generic_params(type_text: &str) -> String {
    if let Some(start) = type_text.find('<') {
        let after_bracket = &type_text[start + 1..];
        let mut depth = 1;
        let mut end = after_bracket.len();
        for (index, character) in after_bracket.char_indices() {
            match character {
                '<' => depth += 1,
                '>' => {
                    depth -= 1;
                    if depth == 0 {
                        end = index;
                        break;
                    }
                }
                _ => {}
            }
        }
        after_bracket[..end].to_string()
    } else {
        type_text.to_string()
    }
}

fn extract_error_propagation(node: &Node, source: &[u8], error_info: &mut ErrorInfo) {
    if node.kind() == "try_expression" {
        let line = node.start_position().row + 1;
        let expression = node_text(node, source);
        let expression = truncate_string(&expression, 50);
        error_info
            .propagation_points
            .push(PropagationPoint { line, expression });
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        extract_error_propagation(&child, source, error_info);
    }
}

fn truncate_string(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars - 3).collect();
        format!("{}...", truncated)
    }
}

fn extract_error_origins(node: &Node, source: &[u8], error_info: &mut ErrorInfo) {
    match node.kind() {
        "call_expression" => {
            if let Some(function) = node.child_by_field_name("function") {
                let text = node_text(&function, source);
                if text == "Err" {
                    let line = node.start_position().row + 1;
                    let message = extract_call_argument(node, source);
                    error_info.error_origins.push(ErrorOrigin {
                        line,
                        kind: ErrorOriginKind::ErrConstructor,
                        message,
                    });
                }
            }
        }
        "macro_invocation" => {
            if let Some(macro_node) = node.child_by_field_name("macro") {
                let macro_name = node_text(&macro_node, source);
                let line = node.start_position().row + 1;

                let kind = match macro_name.as_str() {
                    "anyhow" => Some(ErrorOriginKind::AnyhowMacro),
                    "bail" => Some(ErrorOriginKind::BailMacro),
                    _ => None,
                };

                if let Some(kind) = kind {
                    let message = extract_macro_argument(node, source);
                    error_info.error_origins.push(ErrorOrigin {
                        line,
                        kind,
                        message,
                    });
                }
            }
        }
        "return_expression" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" && node_text(&child, source) == "None" {
                    let line = node.start_position().row + 1;
                    error_info.error_origins.push(ErrorOrigin {
                        line,
                        kind: ErrorOriginKind::NoneReturn,
                        message: None,
                    });
                }
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        extract_error_origins(&child, source, error_info);
    }
}

fn extract_call_argument(node: &Node, source: &[u8]) -> Option<String> {
    if let Some(args) = node.child_by_field_name("arguments") {
        let text = node_text(&args, source);
        let text = text.trim_start_matches('(').trim_end_matches(')').trim();
        if !text.is_empty() && text.len() <= 100 {
            return Some(text.to_string());
        }
    }
    None
}

fn extract_macro_argument(node: &Node, source: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "token_tree" {
            let text = node_text(&child, source);
            let text = text.trim_start_matches('(').trim_end_matches(')');
            let text = text.trim_start_matches('[').trim_end_matches(']');
            let text = text.trim_start_matches('{').trim_end_matches('}');
            let text = text.trim();
            if !text.is_empty() && text.len() <= 100 {
                return Some(text.to_string());
            }
        }
    }
    None
}

const MAX_FULL_BODY_CHARS: usize = 2000;
const MAX_TOTAL_SNIPPET_BUDGET: usize = 50_000;

fn capture_function_body(
    body_node: &Node,
    source: &[u8],
    importance_score: u32,
    current_budget: &mut usize,
) -> Option<FunctionBody> {
    if importance_score >= 30 && *current_budget > 0 {
        let body_text = extract_full_body(body_node, source);
        let body_len = body_text.len();

        if body_len <= MAX_FULL_BODY_CHARS && *current_budget >= body_len {
            *current_budget = current_budget.saturating_sub(body_len);
            return Some(FunctionBody {
                full_text: Some(body_text),
                summary: None,
            });
        }

        let summary = extract_body_summary(body_node, source);
        return Some(FunctionBody {
            full_text: None,
            summary: Some(summary),
        });
    }

    if importance_score >= 15 {
        let summary = extract_body_summary(body_node, source);
        return Some(FunctionBody {
            full_text: None,
            summary: Some(summary),
        });
    }

    None
}

fn extract_full_body(node: &Node, source: &[u8]) -> String {
    let text = node_text(node, source);
    normalize_whitespace(&text)
}

fn normalize_whitespace(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return text.to_string();
    }

    let min_indent = lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0);

    lines
        .iter()
        .map(|line| {
            if line.len() > min_indent {
                &line[min_indent..]
            } else {
                line.trim()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn extract_body_summary(node: &Node, source: &[u8]) -> BodySummary {
    let line_count = compute_line_count(node) as usize;
    let mut statement_count = 0;
    let mut early_returns = Vec::new();
    let mut key_calls = Vec::new();

    collect_body_summary_info(
        node,
        source,
        &mut statement_count,
        &mut early_returns,
        &mut key_calls,
    );

    early_returns.truncate(5);
    key_calls.truncate(10);

    BodySummary {
        line_count,
        statement_count,
        early_returns,
        key_calls,
    }
}

fn collect_body_summary_info(
    node: &Node,
    source: &[u8],
    statement_count: &mut usize,
    early_returns: &mut Vec<String>,
    key_calls: &mut Vec<String>,
) {
    match node.kind() {
        "expression_statement" | "let_declaration" => {
            *statement_count += 1;
        }
        "return_expression" => {
            let text = node_text(node, source);
            let short_text = if text.len() > 60 {
                format!("{}...", &text[..57])
            } else {
                text
            };
            early_returns.push(short_text);
        }
        "call_expression" => {
            if let Some(function) = node.child_by_field_name("function") {
                let call_text = node_text(&function, source);
                if !is_trivial_call(&call_text)
                    && key_calls.len() < 10
                    && !key_calls.contains(&call_text)
                {
                    key_calls.push(call_text);
                }
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_body_summary_info(&child, source, statement_count, early_returns, key_calls);
    }
}

fn is_trivial_call(name: &str) -> bool {
    const TRIVIAL: &[&str] = &[
        "unwrap",
        "expect",
        "clone",
        "to_string",
        "to_owned",
        "into",
        "from",
        "as_ref",
        "as_mut",
        "ok",
        "err",
        "some",
        "none",
        "push",
        "pop",
        "insert",
        "remove",
        "get",
        "len",
        "is_empty",
        "iter",
        "collect",
        "map",
        "filter",
        "and_then",
        "or_else",
        "ok_or",
        "ok_or_else",
        "unwrap_or",
        "unwrap_or_else",
        "unwrap_or_default",
        "default",
        "new",
    ];

    let base = name.split("::").last().unwrap_or(name);
    let base = base.split('.').next_back().unwrap_or(base);
    TRIVIAL.contains(&base)
}

fn extract_safety_info(root: &Node, source: &[u8], result: &mut ParsedFile) {
    extract_unsafe_blocks(root, source, None, &mut result.safety);
    extract_panic_points(root, source, None, &mut result.safety);
    extract_unsafe_traits_and_impls(root, source, &mut result.safety);
}

fn extract_unsafe_blocks(
    node: &Node,
    source: &[u8],
    containing_fn: Option<&str>,
    safety: &mut SafetyInfo,
) {
    let current_fn = if node.kind() == "function_item" {
        find_child_text(node, "identifier", source)
    } else {
        containing_fn.map(|s| s.to_string())
    };

    if node.kind() == "unsafe_block" {
        let line = node.start_position().row + 1;
        let mut operations = Vec::new();
        collect_unsafe_operations(node, source, &mut operations);
        safety.unsafe_blocks.push(UnsafeBlock {
            line,
            containing_function: current_fn.clone(),
            operations,
        });
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        extract_unsafe_blocks(&child, source, current_fn.as_deref(), safety);
    }
}

fn collect_unsafe_operations(node: &Node, source: &[u8], operations: &mut Vec<UnsafeOperation>) {
    match node.kind() {
        "dereference_expression" => {
            let text = node_text(node, source);
            if text.starts_with('*') {
                operations.push(UnsafeOperation::RawPointerDeref);
            }
        }
        "call_expression" => {
            if let Some(func) = node.child_by_field_name("function") {
                let text = node_text(&func, source);
                if text.contains("::") && !text.starts_with("std::") && !text.starts_with("core::")
                {
                    operations.push(UnsafeOperation::UnsafeFunctionCall(text));
                }
            }
        }
        "field_expression" => {
            let text = node_text(node, source);
            if text.contains("union") {
                operations.push(UnsafeOperation::UnionFieldAccess);
            }
        }
        "asm_item" | "asm_block" => {
            operations.push(UnsafeOperation::InlineAssembly);
        }
        "identifier" => {
            let text = node_text(node, source);
            if text.chars().all(|c| c.is_uppercase() || c == '_') && text.len() > 1 {
                if let Some(parent) = node.parent() {
                    if parent.kind() == "assignment_expression"
                        || parent.kind() == "compound_assignment_expr"
                    {
                        operations.push(UnsafeOperation::MutableStaticAccess(text));
                    }
                }
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_unsafe_operations(&child, source, operations);
    }
}

fn extract_panic_points(
    node: &Node,
    source: &[u8],
    containing_fn: Option<&str>,
    safety: &mut SafetyInfo,
) {
    let current_fn = if node.kind() == "function_item" {
        find_child_text(node, "identifier", source)
    } else {
        containing_fn.map(|s| s.to_string())
    };

    let line = node.start_position().row + 1;

    match node.kind() {
        "call_expression" => {
            if let Some(func) = node.child_by_field_name("function") {
                let text = node_text(&func, source);
                if text.ends_with(".unwrap") || text == "unwrap" {
                    safety.panic_points.push(PanicPoint {
                        line,
                        kind: PanicKind::Unwrap,
                        containing_function: current_fn.clone(),
                        context: Some(node_text(node, source)),
                    });
                } else if text.ends_with(".expect") || text == "expect" {
                    let msg = node
                        .child_by_field_name("arguments")
                        .and_then(|args| args.child(1))
                        .map(|arg| node_text(&arg, source))
                        .unwrap_or_default()
                        .trim_matches('"')
                        .to_string();
                    safety.panic_points.push(PanicPoint {
                        line,
                        kind: PanicKind::Expect(msg),
                        containing_function: current_fn.clone(),
                        context: Some(node_text(node, source)),
                    });
                }
            }
        }
        "macro_invocation" => {
            let text = node_text(node, source);
            let kind = if text.starts_with("panic!") {
                Some(PanicKind::PanicMacro)
            } else if text.starts_with("unreachable!") {
                Some(PanicKind::UnreachableMacro)
            } else if text.starts_with("todo!") {
                Some(PanicKind::TodoMacro)
            } else if text.starts_with("unimplemented!") {
                Some(PanicKind::UnimplementedMacro)
            } else if text.starts_with("assert!")
                || text.starts_with("assert_eq!")
                || text.starts_with("assert_ne!")
            {
                Some(PanicKind::Assert)
            } else {
                None
            };
            if let Some(kind) = kind {
                safety.panic_points.push(PanicPoint {
                    line,
                    kind,
                    containing_function: current_fn.clone(),
                    context: Some(text),
                });
            }
        }
        "index_expression" => {
            let text = node_text(node, source);
            if !text.contains("get(") && !text.contains("get_mut(") {
                safety.panic_points.push(PanicPoint {
                    line,
                    kind: PanicKind::IndexAccess,
                    containing_function: current_fn.clone(),
                    context: Some(text),
                });
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        extract_panic_points(&child, source, current_fn.as_deref(), safety);
    }
}

fn extract_unsafe_traits_and_impls(root: &Node, source: &[u8], safety: &mut SafetyInfo) {
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        match child.kind() {
            "trait_item" => {
                if has_modifier(&child, "unsafe") {
                    if let Some(name) = find_child_text(&child, "type_identifier", source) {
                        safety.unsafe_traits.push(name);
                    }
                }
            }
            "impl_item" => {
                if has_modifier(&child, "unsafe") {
                    let trait_name = child
                        .child_by_field_name("trait")
                        .map(|n| node_text(&n, source))
                        .unwrap_or_default();
                    let type_name = child
                        .child_by_field_name("type")
                        .map(|n| node_text(&n, source))
                        .unwrap_or_default();
                    let line = child.start_position().row + 1;
                    safety.unsafe_impls.push(UnsafeImpl {
                        trait_name,
                        type_name,
                        line,
                    });
                }
            }
            _ => extract_unsafe_traits_and_impls(&child, source, safety),
        }
    }
}

fn extract_lifetime_info(root: &Node, source: &[u8], result: &mut ParsedFile) {
    extract_struct_lifetimes(root, source, &mut result.lifetimes);
    extract_function_lifetimes(root, source, None, &mut result.lifetimes);
    extract_complex_bounds(root, source, &mut result.lifetimes);
}

fn extract_struct_lifetimes(node: &Node, source: &[u8], lifetimes: &mut LifetimeInfo) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "struct_item" || child.kind() == "enum_item" {
            let name = find_child_text(&child, "type_identifier", source).unwrap_or_default();
            let line = child.start_position().row + 1;
            let generics = extract_generics(&child, source);

            let mut found_lifetimes = Vec::new();
            let mut has_static = false;

            for segment in generics.split(['<', '>', ',']) {
                let segment = segment.trim();
                if segment.starts_with('\'') {
                    if segment == "'static" {
                        has_static = true;
                    } else {
                        found_lifetimes.push(segment.to_string());
                    }
                }
            }

            if !found_lifetimes.is_empty() || has_static {
                lifetimes.struct_lifetimes.push(StructLifetime {
                    type_name: name,
                    line,
                    lifetimes: found_lifetimes,
                    has_static,
                });
            }
        } else {
            extract_struct_lifetimes(&child, source, lifetimes);
        }
    }
}

fn extract_function_lifetimes(
    node: &Node,
    source: &[u8],
    impl_type: Option<&str>,
    lifetimes: &mut LifetimeInfo,
) {
    let current_impl = if node.kind() == "impl_item" {
        node.child_by_field_name("type")
            .map(|n| node_text(&n, source))
    } else {
        impl_type.map(|s| s.to_string())
    };

    if node.kind() == "function_item" || node.kind() == "function_signature_item" {
        let name = find_child_text(node, "identifier", source).unwrap_or_default();
        let line = node.start_position().row + 1;
        let generics = extract_generics(node, source);
        let signature = extract_function_signature(node, source);

        let mut found_lifetimes = Vec::new();
        let mut has_static = false;
        let mut borrows = Vec::new();

        for segment in generics.split(['<', '>', ',']) {
            let segment = segment.trim();
            if segment.starts_with('\'') {
                if segment == "'static" || segment.starts_with("'static") {
                    has_static = true;
                } else if !segment.contains(':') {
                    found_lifetimes.push(segment.to_string());
                }
            }
        }

        for part in signature.split(',') {
            let part = part.trim();
            if part.contains('&') {
                let is_mutable = part.contains("&mut");
                let lifetime = if part.contains("&'") {
                    part.split("&'").nth(1).map(|s| {
                        let end = s
                            .find(|c: char| !c.is_alphanumeric() && c != '_')
                            .unwrap_or(s.len());
                        format!("'{}", &s[..end])
                    })
                } else {
                    None
                };
                let param_name = part.split(':').next().unwrap_or("").trim().to_string();
                if !param_name.is_empty()
                    && param_name != "self"
                    && param_name != "&self"
                    && param_name != "&mut self"
                {
                    borrows.push(BorrowInfo {
                        param_name,
                        is_mutable,
                        lifetime,
                    });
                }
            }
        }

        if !found_lifetimes.is_empty() || has_static || !borrows.is_empty() {
            lifetimes.function_lifetimes.push(FunctionLifetime {
                function_name: name,
                impl_type: current_impl.clone(),
                line,
                lifetimes: found_lifetimes,
                has_static,
                borrows,
            });
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        extract_function_lifetimes(&child, source, current_impl.as_deref(), lifetimes);
    }
}

fn extract_complex_bounds(root: &Node, source: &[u8], lifetimes: &mut LifetimeInfo) {
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        match child.kind() {
            "function_item" | "struct_item" | "enum_item" | "trait_item" | "impl_item" => {
                if let Some(where_clause) = extract_where_clause(&child, source) {
                    if where_clause.contains(':') {
                        let name = find_child_text(&child, "identifier", source)
                            .or_else(|| find_child_text(&child, "type_identifier", source))
                            .unwrap_or_default();
                        let line = child.start_position().row + 1;
                        lifetimes.complex_bounds.push(ComplexBound {
                            item_name: name,
                            line,
                            bounds: where_clause,
                        });
                    }
                }
            }
            _ => {}
        }
        extract_complex_bounds(&child, source, lifetimes);
    }
}

fn extract_async_info(root: &Node, source: &[u8], result: &mut ParsedFile) {
    extract_async_functions(root, source, None, &mut result.async_info);
    extract_blocking_calls(root, source, None, false, &mut result.async_info);
}

fn extract_async_functions(
    node: &Node,
    source: &[u8],
    impl_type: Option<&str>,
    async_info: &mut AsyncInfo,
) {
    let current_impl = if node.kind() == "impl_item" {
        node.child_by_field_name("type")
            .map(|n| node_text(&n, source))
    } else {
        impl_type.map(|s| s.to_string())
    };

    if node.kind() == "function_item" && has_modifier(node, "async") {
        let name = find_child_text(node, "identifier", source).unwrap_or_default();
        let line = node.start_position().row + 1;
        let mut awaits = Vec::new();
        let mut spawns = Vec::new();

        if let Some(body) = node.child_by_field_name("body") {
            collect_await_points(&body, source, &mut awaits);
            collect_spawn_points(&body, source, &mut spawns);
        }

        async_info.async_functions.push(AsyncFunction {
            name,
            impl_type: current_impl.clone(),
            line,
            awaits,
            spawns,
        });
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        extract_async_functions(&child, source, current_impl.as_deref(), async_info);
    }
}

fn collect_await_points(node: &Node, source: &[u8], awaits: &mut Vec<AwaitPoint>) {
    if node.kind() == "await_expression" {
        let line = node.start_position().row + 1;
        let expression = node_text(node, source);
        awaits.push(AwaitPoint { line, expression });
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_await_points(&child, source, awaits);
    }
}

fn collect_spawn_points(node: &Node, source: &[u8], spawns: &mut Vec<SpawnPoint>) {
    if node.kind() == "call_expression" {
        let text = node_text(node, source);
        let line = node.start_position().row + 1;

        let spawn_type = if text.contains("tokio::spawn")
            || text.contains("spawn(") && text.contains("tokio")
        {
            Some(SpawnType::TokioSpawn)
        } else if text.contains("spawn_blocking") {
            Some(SpawnType::TokioSpawnBlocking)
        } else if text.contains("async_std::task::spawn") {
            Some(SpawnType::AsyncStdSpawn)
        } else if text.contains("spawn") && (text.contains("task") || text.contains("runtime")) {
            Some(SpawnType::Other(text.clone()))
        } else {
            None
        };

        if let Some(spawn_type) = spawn_type {
            spawns.push(SpawnPoint { line, spawn_type });
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_spawn_points(&child, source, spawns);
    }
}

fn extract_blocking_calls(
    node: &Node,
    source: &[u8],
    containing_fn: Option<&str>,
    in_async: bool,
    async_info: &mut AsyncInfo,
) {
    let (current_fn, current_async) = if node.kind() == "function_item" {
        let name = find_child_text(node, "identifier", source);
        let is_async = has_modifier(node, "async");
        (name, is_async)
    } else {
        (containing_fn.map(|s| s.to_string()), in_async)
    };

    if node.kind() == "call_expression" {
        let text = node_text(node, source);
        let line = node.start_position().row + 1;

        const BLOCKING_CALLS: &[&str] = &[
            "std::fs::",
            "std::io::",
            "std::net::",
            "std::thread::sleep",
            "thread::sleep",
            ".read(",
            ".write(",
            ".read_to_string",
            ".read_to_end",
            "File::open",
            "File::create",
        ];

        for pattern in BLOCKING_CALLS {
            if text.contains(pattern) {
                async_info.blocking_calls.push(BlockingCall {
                    line,
                    call: text.clone(),
                    in_async_context: current_async,
                    containing_function: current_fn.clone(),
                });
                break;
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        extract_blocking_calls(
            &child,
            source,
            current_fn.as_deref(),
            current_async,
            async_info,
        );
    }
}

fn extract_feature_flags(root: &Node, source: &[u8], result: &mut ParsedFile) {
    let mut feature_map: std::collections::HashMap<String, Vec<GatedSymbol>> =
        std::collections::HashMap::new();

    collect_feature_gated_items(
        root,
        source,
        &mut feature_map,
        &mut result.feature_flags.cfg_blocks,
    );

    for (feature_name, symbols) in feature_map {
        result.feature_flags.feature_gates.push(FeatureGate {
            feature_name,
            symbols,
        });
    }
}

fn collect_feature_gated_items(
    node: &Node,
    source: &[u8],
    feature_map: &mut std::collections::HashMap<String, Vec<GatedSymbol>>,
    cfg_blocks: &mut Vec<CfgBlock>,
) {
    let mut cursor = node.walk();
    let children: Vec<_> = node.children(&mut cursor).collect();

    let mut index = 0;
    while index < children.len() {
        let child = &children[index];

        if child.kind() == "attribute_item" {
            let text = node_text(child, source);

            if text.contains("#[cfg(feature") {
                if let Some(feature) = extract_feature_name(&text) {
                    if index + 1 < children.len() {
                        let next = &children[index + 1];
                        let (name, kind) = get_item_name_and_kind(next, source);
                        if !name.is_empty() {
                            let line = next.start_position().row + 1;
                            feature_map.entry(feature).or_default().push(GatedSymbol {
                                name,
                                kind,
                                line,
                            });
                        }
                    }
                }
            } else if text.contains("#[cfg(") {
                if let Some(condition) = extract_cfg_condition(&text) {
                    let line = child.start_position().row + 1;
                    let mut affected_items = Vec::new();

                    if index + 1 < children.len() {
                        let next = &children[index + 1];
                        let (name, _) = get_item_name_and_kind(next, source);
                        if !name.is_empty() {
                            affected_items.push(name);
                        }
                    }

                    cfg_blocks.push(CfgBlock {
                        condition,
                        line,
                        affected_items,
                    });
                }
            }
        }

        collect_feature_gated_items(child, source, feature_map, cfg_blocks);
        index += 1;
    }
}

fn extract_feature_name(attr_text: &str) -> Option<String> {
    let start = attr_text.find("feature = \"")?;
    let rest = &attr_text[start + 11..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn extract_cfg_condition(attr_text: &str) -> Option<String> {
    let start = attr_text.find("#[cfg(")?;
    let rest = &attr_text[start + 6..];
    let end = rest.rfind(")]")?;
    Some(rest[..end].to_string())
}

fn get_item_name_and_kind(node: &Node, source: &[u8]) -> (String, String) {
    let kind = match node.kind() {
        "function_item" => "fn",
        "struct_item" => "struct",
        "enum_item" => "enum",
        "trait_item" => "trait",
        "impl_item" => "impl",
        "const_item" => "const",
        "static_item" => "static",
        "type_item" => "type",
        "mod_item" => "mod",
        "macro_definition" => "macro",
        _ => return (String::new(), String::new()),
    };

    let name = find_child_text(node, "identifier", source)
        .or_else(|| find_child_text(node, "type_identifier", source))
        .unwrap_or_default();

    (name, kind.to_string())
}

fn extract_doc_comments(root: &Node, source: &[u8], result: &mut ParsedFile) {
    collect_item_docs(root, source, &mut result.doc_info.item_docs);
}

fn collect_item_docs(node: &Node, source: &[u8], docs: &mut Vec<ItemDoc>) {
    let mut cursor = node.walk();
    let children: Vec<_> = node.children(&mut cursor).collect();

    let mut index = 0;
    while index < children.len() {
        let child = &children[index];

        if child.kind() == "line_comment" || child.kind() == "block_comment" {
            let text = node_text(child, source);

            if text.starts_with("///") || text.starts_with("/**") {
                let mut doc_lines = Vec::new();
                let mut doc_index = index;

                while doc_index < children.len() {
                    let doc_node = &children[doc_index];
                    let doc_text = node_text(doc_node, source);

                    if doc_text.starts_with("///") {
                        doc_lines.push(
                            doc_text
                                .strip_prefix("///")
                                .unwrap_or("")
                                .trim()
                                .to_string(),
                        );
                        doc_index += 1;
                    } else if doc_text.starts_with("/**") {
                        let content = doc_text
                            .strip_prefix("/**")
                            .and_then(|s| s.strip_suffix("*/"))
                            .unwrap_or("");
                        doc_lines.push(content.trim().to_string());
                        doc_index += 1;
                        break;
                    } else {
                        break;
                    }
                }

                if doc_index < children.len() {
                    let item = &children[doc_index];
                    let (name, kind) = get_item_name_and_kind(item, source);

                    if !name.is_empty() {
                        let full_doc = doc_lines.join(" ");
                        let summary = full_doc
                            .split('.')
                            .next()
                            .unwrap_or(&full_doc)
                            .trim()
                            .to_string();
                        let line = item.start_position().row + 1;

                        docs.push(ItemDoc {
                            item_name: name,
                            item_kind: kind,
                            line,
                            summary,
                            has_examples: full_doc.contains("# Example")
                                || full_doc.contains("```"),
                            has_panics_section: full_doc.contains("# Panic"),
                            has_safety_section: full_doc.contains("# Safety"),
                            has_errors_section: full_doc.contains("# Error"),
                        });
                    }
                }

                index = doc_index;
                continue;
            }
        }

        collect_item_docs(child, source, docs);
        index += 1;
    }
}

fn extract_generic_constraints(root: &Node, source: &[u8], result: &mut ParsedFile) {
    collect_generic_constraints(root, source, &mut result.generic_constraints.constraints);
}

fn collect_generic_constraints(node: &Node, source: &[u8], constraints: &mut Vec<ItemConstraints>) {
    match node.kind() {
        "function_item" | "struct_item" | "enum_item" | "trait_item" | "impl_item" => {
            let (name, kind) = get_item_name_and_kind(node, source);
            let line = node.start_position().row + 1;
            let generics = extract_generics(node, source);
            let where_clause = extract_where_clause(node, source);

            if !generics.is_empty() || where_clause.is_some() {
                let type_params = parse_type_params(&generics);

                if !type_params.is_empty() || where_clause.is_some() {
                    constraints.push(ItemConstraints {
                        item_name: name,
                        item_kind: kind,
                        line,
                        type_params,
                        where_clause,
                    });
                }
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_generic_constraints(&child, source, constraints);
    }
}

fn parse_type_params(generics: &str) -> Vec<TypeParam> {
    let mut params = Vec::new();
    let content = generics.trim_start_matches('<').trim_end_matches('>');

    let mut depth = 0;
    let mut current_param = String::new();

    for c in content.chars() {
        match c {
            '<' => {
                depth += 1;
                current_param.push(c);
            }
            '>' => {
                depth -= 1;
                current_param.push(c);
            }
            ',' if depth == 0 => {
                if let Some(param) = parse_single_type_param(&current_param) {
                    params.push(param);
                }
                current_param.clear();
            }
            _ => current_param.push(c),
        }
    }

    if let Some(param) = parse_single_type_param(&current_param) {
        params.push(param);
    }

    params
}

fn parse_single_type_param(param: &str) -> Option<TypeParam> {
    let param = param.trim();
    if param.is_empty() || param.starts_with('\'') {
        return None;
    }

    let parts: Vec<&str> = param.splitn(2, ':').collect();
    let name = parts[0].trim().to_string();

    if name.is_empty() {
        return None;
    }

    let bounds = if parts.len() > 1 {
        parts[1]
            .split('+')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    } else {
        Vec::new()
    };

    Some(TypeParam { name, bounds })
}

fn extract_test_info(root: &Node, source: &[u8], result: &mut ParsedFile) {
    collect_test_functions(root, source, &mut result.test_info);
    collect_test_modules(root, source, &mut result.test_info);
    infer_tested_items(&mut result.test_info);
}

fn collect_test_functions(node: &Node, source: &[u8], test_info: &mut TestInfo) {
    let mut cursor = node.walk();
    let children: Vec<_> = node.children(&mut cursor).collect();

    for index in 0..children.len() {
        let child = &children[index];

        if child.kind() == "function_item" {
            let mut is_test = false;
            let mut is_ignored = false;
            let mut should_panic = false;
            let is_async = has_modifier(child, "async");

            for prev_index in (0..index).rev() {
                let prev = &children[prev_index];
                if prev.kind() != "attribute_item" {
                    break;
                }
                let attr_text = node_text(prev, source);
                if attr_text.contains("#[test]")
                    || attr_text.contains("#[tokio::test")
                    || attr_text.contains("#[async_std::test")
                {
                    is_test = true;
                }
                if attr_text.contains("#[ignore") {
                    is_ignored = true;
                }
                if attr_text.contains("#[should_panic") {
                    should_panic = true;
                }
            }

            if is_test {
                let name = find_child_text(child, "identifier", source).unwrap_or_default();
                let line = child.start_position().row + 1;
                let tested_function = infer_tested_function(&name);

                test_info.test_functions.push(TestFunction {
                    name,
                    line,
                    is_async,
                    is_ignored,
                    should_panic,
                    tested_function,
                });
            }
        }

        collect_test_functions(child, source, test_info);
    }
}

fn collect_test_modules(node: &Node, source: &[u8], test_info: &mut TestInfo) {
    let mut cursor = node.walk();
    let children: Vec<_> = node.children(&mut cursor).collect();

    for index in 0..children.len() {
        let child = &children[index];

        if child.kind() == "mod_item" {
            let name = find_child_text(child, "identifier", source).unwrap_or_default();

            let mut is_test_mod = name == "tests" || name == "test";

            for prev_index in (0..index).rev() {
                let prev = &children[prev_index];
                if prev.kind() != "attribute_item" {
                    break;
                }
                let attr_text = node_text(prev, source);
                if attr_text.contains("#[cfg(test)]") {
                    is_test_mod = true;
                    break;
                }
            }

            if is_test_mod {
                let line = child.start_position().row + 1;
                let test_count = count_tests_in_module(child, source);
                test_info.test_modules.push(TestModule {
                    name,
                    line,
                    test_count,
                });
            }
        }

        collect_test_modules(child, source, test_info);
    }
}

fn count_tests_in_module(node: &Node, source: &[u8]) -> usize {
    let mut count = 0;
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "attribute_item" {
            let text = node_text(&child, source);
            if text.contains("#[test]")
                || text.contains("#[tokio::test")
                || text.contains("#[async_std::test")
            {
                count += 1;
            }
        }
        count += count_tests_in_module(&child, source);
    }

    count
}

fn infer_tested_function(test_name: &str) -> Option<String> {
    let name = test_name.strip_prefix("test_")?;

    if name.ends_with("_works") || name.ends_with("_succeeds") || name.ends_with("_success") {
        let base = name
            .strip_suffix("_works")
            .or_else(|| name.strip_suffix("_succeeds"))
            .or_else(|| name.strip_suffix("_success"))
            .unwrap_or(name);
        return Some(base.to_string());
    }

    if name.ends_with("_fails") || name.ends_with("_error") || name.ends_with("_panics") {
        let base = name
            .strip_suffix("_fails")
            .or_else(|| name.strip_suffix("_error"))
            .or_else(|| name.strip_suffix("_panics"))
            .unwrap_or(name);
        return Some(base.to_string());
    }

    Some(name.to_string())
}

fn infer_tested_items(test_info: &mut TestInfo) {
    let mut item_tests: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();

    for test in &test_info.test_functions {
        if let Some(ref tested) = test.tested_function {
            item_tests
                .entry(tested.clone())
                .or_default()
                .push(test.name.clone());
        }
    }

    for (item_name, test_names) in item_tests {
        let coverage_hints = if test_names
            .iter()
            .any(|n| n.contains("error") || n.contains("fail"))
        {
            vec!["error path".to_string()]
        } else {
            vec![]
        };

        test_info.tested_items.push(TestedItem {
            item_name,
            test_names,
            coverage_hints,
        });
    }
}
