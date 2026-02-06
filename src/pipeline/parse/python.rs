use anyhow::{Result, anyhow};
use std::cell::RefCell;
use tree_sitter::{Node, Parser, Tree};

use crate::extract::calls::{CallEdge, CallInfo};
use crate::extract::complexity::{ComplexityMetrics, FunctionComplexity};
use crate::extract::errors::{
    ErrorInfo, ErrorOrigin, ErrorOriginKind, ErrorReturnType, PropagationPoint,
};
use crate::extract::imports::{ImportInfo, ImportKind};
use crate::extract::safety::{
    AsyncFunction, AwaitPoint, PanicKind, PanicPoint, PythonDangerousCall, RiskLevel, TestFunction,
    TestInfo, TestModule,
};
use crate::extract::symbols::{
    ClassField, ClassMethod, DecoratorInfo, FunctionBody, Parameter, ParameterKind, Symbol,
    SymbolKind, Visibility,
};
use crate::pipeline::parse::{CapturedBody, ParsedFile};

thread_local! {
    static PYTHON_PARSER: RefCell<Parser> = RefCell::new({
        let mut parser = Parser::new();
        parser.set_language(&tree_sitter_python::LANGUAGE.into()).expect("Python grammar");
        parser.set_timeout_micros(10_000_000);
        parser
    });
}

pub fn parse_python_file(content: &str, file_path: &str) -> Result<ParsedFile> {
    PYTHON_PARSER.with(|parser| {
        let mut parser = parser.borrow_mut();
        let tree = parser
            .parse(content, None)
            .ok_or_else(|| anyhow!("Failed to parse Python file"))?;

        extract_from_tree(&tree, content, file_path)
    })
}

fn extract_from_tree(tree: &Tree, source: &str, file_path: &str) -> Result<ParsedFile> {
    let root = tree.root_node();
    let source_bytes = source.as_bytes();

    let mut result = ParsedFile::default();

    extract_module_docstring(&root, source_bytes, &mut result);
    extract_imports(&root, source_bytes, &mut result);
    extract_items(&root, source_bytes, file_path, &mut result);
    extract_identifier_locations(&root, source_bytes, &mut result);
    extract_test_info(&root, source_bytes, &mut result);

    Ok(result)
}

fn extract_module_docstring(root: &Node, source: &[u8], result: &mut ParsedFile) {
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        if child.kind() == "expression_statement" {
            if let Some(string_node) = child.child(0) {
                if string_node.kind() == "string" {
                    let text = node_text(&string_node, source);
                    let doc = extract_string_content(&text);
                    if !doc.is_empty() {
                        result.module_doc = Some(doc);
                    }
                    return;
                }
            }
        } else if child.kind() != "comment" {
            break;
        }
    }
}

fn extract_imports(root: &Node, source: &[u8], result: &mut ParsedFile) {
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        match child.kind() {
            "import_statement" => {
                let line = child.start_position().row + 1;
                let text = node_text(&child, source);
                let module = text.strip_prefix("import ").unwrap_or(&text).trim();
                result.imports.push(ImportInfo {
                    path: module.to_string(),
                    line,
                    kind: ImportKind::PythonImport {
                        module: module.to_string(),
                    },
                });
            }
            "import_from_statement" => {
                let line = child.start_position().row + 1;
                let module = child
                    .child_by_field_name("module_name")
                    .map(|n| node_text(&n, source))
                    .unwrap_or_default();

                let mut names = Vec::new();
                let mut name_cursor = child.walk();
                for name_child in child.children(&mut name_cursor) {
                    if name_child.kind() == "dotted_name" || name_child.kind() == "aliased_import" {
                        names.push(node_text(&name_child, source));
                    }
                }

                result.imports.push(ImportInfo {
                    path: format!("{}.{}", module, names.join(", ")),
                    line,
                    kind: ImportKind::PythonFromImport { module, names },
                });
            }
            _ => {}
        }
    }
}

fn extract_items(node: &Node, source: &[u8], file_path: &str, result: &mut ParsedFile) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_definition" => {
                extract_class(&child, source, file_path, result);
            }
            "function_definition" | "decorated_definition" => {
                extract_function(&child, source, file_path, None, result);
            }
            "expression_statement" => {
                extract_module_level_assignment(&child, source, result);
            }
            _ => {}
        }
    }
}

fn extract_class(node: &Node, source: &[u8], file_path: &str, result: &mut ParsedFile) {
    let (class_node, decorators) = if node.kind() == "decorated_definition" {
        let decs = extract_decorators(node, source);
        let inner = node
            .children(&mut node.walk())
            .find(|c| c.kind() == "class_definition");
        match inner {
            Some(c) => (c, decs),
            None => return,
        }
    } else {
        (*node, Vec::new())
    };

    let name = class_node
        .child_by_field_name("name")
        .map(|n| node_text(&n, source))
        .unwrap_or_default();

    let line = class_node.start_position().row + 1;
    let visibility = Visibility::from_python_name(&name);

    let mut bases = Vec::new();
    if let Some(args) = class_node.child_by_field_name("superclasses") {
        let mut arg_cursor = args.walk();
        for arg in args.children(&mut arg_cursor) {
            if arg.kind() == "identifier" || arg.kind() == "attribute" {
                bases.push(node_text(&arg, source));
            }
        }
    }

    let is_dataclass = decorators.iter().any(|d| {
        d.name == "dataclass" || d.name == "dataclasses.dataclass" || d.name.ends_with(".dataclass")
    });

    let is_protocol = bases
        .iter()
        .any(|b| b == "Protocol" || b.ends_with(".Protocol"));

    let is_abc = bases
        .iter()
        .any(|b| b == "ABC" || b.ends_with(".ABC") || b == "ABCMeta");

    let mut fields = Vec::new();
    let mut methods = Vec::new();

    if let Some(body) = class_node.child_by_field_name("body") {
        extract_class_body(
            &body,
            source,
            file_path,
            &name,
            &mut fields,
            &mut methods,
            result,
        );
    }

    result.symbols.symbols.push(Symbol {
        name: name.clone(),
        kind: SymbolKind::Class {
            bases,
            fields,
            methods,
            decorators,
            is_dataclass,
            is_protocol,
            is_abc,
        },
        visibility,
        generics: String::new(),
        line,
        is_async: false,
        is_unsafe: false,
        is_const: false,
        re_exported_as: None,
    });

    for imp in &result.symbols.impl_map.clone() {
        if imp.1 == name {
            continue;
        }
    }

    for base in &result
        .symbols
        .symbols
        .last()
        .map(|s| {
            if let SymbolKind::Class { bases, .. } = &s.kind {
                bases.clone()
            } else {
                Vec::new()
            }
        })
        .unwrap_or_default()
    {
        result.symbols.impl_map.push((base.clone(), name.clone()));
    }
}

fn extract_class_body(
    body: &Node,
    source: &[u8],
    file_path: &str,
    class_name: &str,
    fields: &mut Vec<ClassField>,
    methods: &mut Vec<ClassMethod>,
    result: &mut ParsedFile,
) {
    let mut cursor = body.walk();

    for child in body.children(&mut cursor) {
        match child.kind() {
            "function_definition" | "decorated_definition" => {
                extract_method(&child, source, file_path, class_name, methods, result);
            }
            "expression_statement" => {
                if let Some(assign) = child.child(0) {
                    if assign.kind() == "assignment" || assign.kind() == "annotated_assignment" {
                        extract_class_field(&assign, source, fields);
                    }
                }
            }
            _ => {}
        }
    }
}

fn extract_class_field(node: &Node, source: &[u8], fields: &mut Vec<ClassField>) {
    let name = node
        .child_by_field_name("left")
        .or_else(|| node.child(0))
        .map(|n| node_text(&n, source))
        .unwrap_or_default();

    if name.is_empty() || name.starts_with("self.") {
        return;
    }

    let type_hint = node
        .child_by_field_name("type")
        .map(|n| node_text(&n, source));

    let default_value = node
        .child_by_field_name("right")
        .or_else(|| node.child_by_field_name("value"))
        .map(|n| {
            let text = node_text(&n, source);
            truncate_string(&text, 50)
        });

    let is_class_var = type_hint.as_ref().is_some_and(|t| t.contains("ClassVar"));

    fields.push(ClassField {
        name,
        type_hint,
        default_value,
        is_class_var,
    });
}

fn extract_method(
    node: &Node,
    source: &[u8],
    file_path: &str,
    class_name: &str,
    methods: &mut Vec<ClassMethod>,
    result: &mut ParsedFile,
) {
    let (func_node, decorators) = if node.kind() == "decorated_definition" {
        let decs = extract_decorators(node, source);
        let inner = node
            .children(&mut node.walk())
            .find(|c| c.kind() == "function_definition");
        match inner {
            Some(f) => (f, decs),
            None => return,
        }
    } else {
        (*node, Vec::new())
    };

    let name = func_node
        .child_by_field_name("name")
        .map(|n| node_text(&n, source))
        .unwrap_or_default();

    let line = func_node.start_position().row + 1;
    let visibility = Visibility::from_python_name(&name);

    let is_async = node.kind() == "async_function_definition"
        || func_node.kind() == "async_function_definition"
        || node.children(&mut node.walk()).any(|c| c.kind() == "async");

    let is_classmethod = decorators.iter().any(|d| d.name == "classmethod");
    let is_staticmethod = decorators.iter().any(|d| d.name == "staticmethod");
    let is_property = decorators.iter().any(|d| {
        d.name == "property" || d.name.ends_with(".setter") || d.name.ends_with(".getter")
    });
    let is_abstract = decorators
        .iter()
        .any(|d| d.name == "abstractmethod" || d.name == "abc.abstractmethod");

    let parameters = extract_parameters(&func_node, source);
    let return_type = func_node
        .child_by_field_name("return_type")
        .map(|n| node_text(&n, source));

    let signature = format_python_signature(&parameters, return_type.as_deref());

    let docstring = extract_function_docstring(&func_node, source);

    methods.push(ClassMethod {
        name: name.clone(),
        visibility: visibility.clone(),
        signature: signature.clone(),
        is_async,
        is_classmethod,
        is_staticmethod,
        is_property,
        is_abstract,
        line,
        docstring: docstring.clone(),
    });

    if let Some(body) = func_node.child_by_field_name("body") {
        let complexity = compute_cyclomatic_complexity(&body, source);
        let line_count = compute_line_count(&body);

        let importance_score = (complexity * 2)
            + (line_count / 10)
            + if matches!(visibility, Visibility::Public) {
                10
            } else {
                0
            }
            + if name.starts_with("test_") { 0 } else { 5 };

        result.complexity.push(FunctionComplexity {
            name: format!("{}.{}", class_name, name),
            impl_type: Some(class_name.to_string()),
            line,
            metrics: ComplexityMetrics {
                cyclomatic: complexity,
                line_count,
                nesting_depth: compute_nesting_depth(&body),
                call_sites: 0,
                churn_score: 0,
                is_public: matches!(visibility, Visibility::Public),
                is_test: name.starts_with("test_"),
            },
        });

        extract_calls_from_body(
            &body,
            source,
            file_path,
            &format!("{}.{}", class_name, name),
            Some(class_name),
            result,
        );
        extract_safety_from_body(&body, source, Some(&name), result);
        extract_error_info(
            &body,
            source,
            file_path,
            &name,
            Some(class_name),
            line,
            result,
        );

        if importance_score >= 15 && !name.starts_with("test_") && !is_dunder_method(&name) {
            let body_text = node_text(&body, source);
            result.captured_bodies.push(CapturedBody {
                function_name: name.clone(),
                impl_type: Some(class_name.to_string()),
                line,
                body: FunctionBody {
                    full_text: if importance_score >= 30 {
                        Some(body_text)
                    } else {
                        None
                    },
                    summary: if importance_score < 30 {
                        Some(crate::extract::symbols::BodySummary {
                            line_count: line_count as usize,
                            statement_count: count_statements(&body),
                            early_returns: collect_early_returns(&body, source),
                            key_calls: collect_key_calls(&body, source),
                        })
                    } else {
                        None
                    },
                },
                importance_score,
            });
        }
    }
}

fn extract_function(
    node: &Node,
    source: &[u8],
    file_path: &str,
    impl_type: Option<&str>,
    result: &mut ParsedFile,
) {
    let (func_node, decorators) = if node.kind() == "decorated_definition" {
        let decs = extract_decorators(node, source);
        let inner = node
            .children(&mut node.walk())
            .find(|c| c.kind() == "function_definition" || c.kind() == "async_function_definition");
        match inner {
            Some(f) => (f, decs),
            None => return,
        }
    } else {
        (*node, Vec::new())
    };

    let name = func_node
        .child_by_field_name("name")
        .map(|n| node_text(&n, source))
        .unwrap_or_default();

    let line = func_node.start_position().row + 1;
    let visibility = Visibility::from_python_name(&name);

    let is_async = func_node.kind() == "async_function_definition";
    let is_generator = check_is_generator(&func_node);

    let is_classmethod = decorators.iter().any(|d| d.name == "classmethod");
    let is_staticmethod = decorators.iter().any(|d| d.name == "staticmethod");
    let is_property = decorators.iter().any(|d| d.name == "property");

    let parameters = extract_parameters(&func_node, source);
    let return_type = func_node
        .child_by_field_name("return_type")
        .map(|n| node_text(&n, source));

    let docstring = extract_function_docstring(&func_node, source);

    result.symbols.symbols.push(Symbol {
        name: name.clone(),
        kind: SymbolKind::PythonFunction {
            parameters,
            return_type,
            decorators,
            is_generator,
            is_classmethod,
            is_staticmethod,
            is_property,
            docstring: docstring.clone(),
        },
        visibility: visibility.clone(),
        generics: String::new(),
        line,
        is_async,
        is_unsafe: false,
        is_const: false,
        re_exported_as: None,
    });

    if let Some(body) = func_node.child_by_field_name("body") {
        let complexity = compute_cyclomatic_complexity(&body, source);
        let line_count = compute_line_count(&body);

        let importance_score = (complexity * 2)
            + (line_count / 10)
            + if matches!(visibility, Visibility::Public) {
                10
            } else {
                0
            }
            + if name.starts_with("test_") { 0 } else { 5 };

        result.complexity.push(FunctionComplexity {
            name: name.clone(),
            impl_type: impl_type.map(|s| s.to_string()),
            line,
            metrics: ComplexityMetrics {
                cyclomatic: complexity,
                line_count,
                nesting_depth: compute_nesting_depth(&body),
                call_sites: 0,
                churn_score: 0,
                is_public: matches!(visibility, Visibility::Public),
                is_test: name.starts_with("test_"),
            },
        });

        extract_calls_from_body(&body, source, file_path, &name, impl_type, result);
        extract_safety_from_body(&body, source, Some(&name), result);
        extract_error_info(&body, source, file_path, &name, impl_type, line, result);

        if importance_score >= 15 && !name.starts_with("test_") {
            let body_text = node_text(&body, source);
            result.captured_bodies.push(CapturedBody {
                function_name: name.clone(),
                impl_type: impl_type.map(|s| s.to_string()),
                line,
                body: FunctionBody {
                    full_text: if importance_score >= 30 {
                        Some(body_text)
                    } else {
                        None
                    },
                    summary: if importance_score < 30 {
                        Some(crate::extract::symbols::BodySummary {
                            line_count: line_count as usize,
                            statement_count: count_statements(&body),
                            early_returns: collect_early_returns(&body, source),
                            key_calls: collect_key_calls(&body, source),
                        })
                    } else {
                        None
                    },
                },
                importance_score,
            });
        }

        if is_async {
            let mut awaits = Vec::new();
            collect_await_points(&body, source, &mut awaits);
            if !awaits.is_empty() {
                result.async_info.async_functions.push(AsyncFunction {
                    name: name.clone(),
                    impl_type: impl_type.map(|s| s.to_string()),
                    line,
                    awaits,
                    spawns: Vec::new(),
                });
            }
        }
    }
}

fn extract_module_level_assignment(node: &Node, source: &[u8], result: &mut ParsedFile) {
    if let Some(assign) = node.child(0) {
        if assign.kind() != "assignment" && assign.kind() != "annotated_assignment" {
            return;
        }

        let name = assign
            .child_by_field_name("left")
            .or_else(|| assign.child(0))
            .map(|n| node_text(&n, source))
            .unwrap_or_default();

        if name.is_empty() || name.contains('.') {
            return;
        }

        let line = node.start_position().row + 1;
        let visibility = Visibility::from_python_name(&name);

        let type_hint = assign
            .child_by_field_name("type")
            .map(|n| node_text(&n, source));

        let value = assign
            .child_by_field_name("right")
            .or_else(|| assign.child_by_field_name("value"))
            .and_then(|n| {
                let text = node_text(&n, source);
                if text.len() > 80 { None } else { Some(text) }
            });

        result.symbols.symbols.push(Symbol {
            name,
            kind: SymbolKind::Variable { type_hint, value },
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

fn extract_decorators(node: &Node, source: &[u8]) -> Vec<DecoratorInfo> {
    let mut decorators = Vec::new();
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "decorator" {
            let text = node_text(&child, source);
            let text = text.strip_prefix('@').unwrap_or(&text);

            let (name, arguments) = if let Some(paren_pos) = text.find('(') {
                let name = text[..paren_pos].trim().to_string();
                let args = text[paren_pos..].trim().to_string();
                (name, Some(args))
            } else {
                (text.trim().to_string(), None)
            };

            decorators.push(DecoratorInfo { name, arguments });
        }
    }

    decorators
}

fn extract_parameters(node: &Node, source: &[u8]) -> Vec<Parameter> {
    let mut params = Vec::new();

    let parameters = match node.child_by_field_name("parameters") {
        Some(p) => p,
        None => return params,
    };

    let mut cursor = parameters.walk();
    let mut seen_star = false;
    let mut seen_slash = false;

    for child in parameters.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                let name = node_text(&child, source);
                let kind = if seen_star {
                    ParameterKind::KeywordOnly
                } else if !seen_slash {
                    ParameterKind::PositionalOnly
                } else {
                    ParameterKind::Regular
                };
                params.push(Parameter {
                    name,
                    type_hint: None,
                    default_value: None,
                    kind,
                });
            }
            "typed_parameter" | "default_parameter" | "typed_default_parameter" => {
                let name = child
                    .child_by_field_name("name")
                    .or_else(|| child.child(0))
                    .map(|n| node_text(&n, source))
                    .unwrap_or_default();

                let type_hint = child
                    .child_by_field_name("type")
                    .map(|n| node_text(&n, source));

                let default_value = child
                    .child_by_field_name("value")
                    .map(|n| node_text(&n, source));

                let kind = if seen_star {
                    ParameterKind::KeywordOnly
                } else if !seen_slash {
                    ParameterKind::PositionalOnly
                } else {
                    ParameterKind::Regular
                };

                params.push(Parameter {
                    name,
                    type_hint,
                    default_value,
                    kind,
                });
            }
            "list_splat_pattern" | "dictionary_splat_pattern" => {
                let name = node_text(&child, source);
                let kind = if name.starts_with("**") {
                    ParameterKind::Kwargs
                } else {
                    ParameterKind::Args
                };
                let clean_name = name.trim_start_matches('*').to_string();
                params.push(Parameter {
                    name: clean_name,
                    type_hint: None,
                    default_value: None,
                    kind,
                });
            }
            "*" => seen_star = true,
            "/" => seen_slash = true,
            _ => {}
        }
    }

    params
}

fn format_python_signature(params: &[Parameter], return_type: Option<&str>) -> String {
    let param_strs: Vec<String> = params
        .iter()
        .map(|p| {
            let prefix = match p.kind {
                ParameterKind::Args => "*",
                ParameterKind::Kwargs => "**",
                _ => "",
            };
            let type_part = p
                .type_hint
                .as_ref()
                .map(|t| format!(": {}", t))
                .unwrap_or_default();
            let default_part = p
                .default_value
                .as_ref()
                .map(|d| format!(" = {}", d))
                .unwrap_or_default();
            format!("{}{}{}{}", prefix, p.name, type_part, default_part)
        })
        .collect();

    let params_str = param_strs.join(", ");
    match return_type {
        Some(rt) => format!("({}) -> {}", params_str, rt),
        None => format!("({})", params_str),
    }
}

fn extract_function_docstring(node: &Node, source: &[u8]) -> Option<String> {
    let body = node.child_by_field_name("body")?;
    let mut cursor = body.walk();

    if let Some(child) = body.children(&mut cursor).next() {
        if child.kind() == "expression_statement" {
            if let Some(string_node) = child.child(0) {
                if string_node.kind() == "string" {
                    let text = node_text(&string_node, source);
                    let doc = extract_string_content(&text);
                    if !doc.is_empty() {
                        let first_line = doc.lines().next().unwrap_or(&doc);
                        return Some(first_line.to_string());
                    }
                }
            }
        }
    }

    None
}

fn check_is_generator(node: &Node) -> bool {
    if let Some(body) = node.child_by_field_name("body") {
        return contains_yield(&body);
    }
    false
}

fn contains_yield(node: &Node) -> bool {
    if node.kind() == "yield" || node.kind() == "yield_expression" {
        return true;
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "function_definition" || child.kind() == "async_function_definition" {
            continue;
        }
        if contains_yield(&child) {
            return true;
        }
    }
    false
}

fn compute_cyclomatic_complexity(node: &Node, source: &[u8]) -> u32 {
    let mut complexity = 1;
    count_branch_points(node, source, &mut complexity);
    complexity
}

fn count_branch_points(node: &Node, source: &[u8], complexity: &mut u32) {
    match node.kind() {
        "if_statement" | "elif_clause" | "while_statement" | "for_statement" => {
            *complexity += 1;
        }
        "match_statement" => {
            let mut cursor = node.walk();
            let case_count = node
                .children(&mut cursor)
                .filter(|c| c.kind() == "case_clause")
                .count();
            *complexity += case_count.saturating_sub(1) as u32;
        }
        "try_statement" => {
            let mut cursor = node.walk();
            let except_count = node
                .children(&mut cursor)
                .filter(|c| c.kind() == "except_clause")
                .count();
            *complexity += except_count as u32;
        }
        "boolean_operator" => {
            let text = node_text(node, source);
            if text.contains(" and ") || text.contains(" or ") {
                *complexity += 1;
            }
        }
        "conditional_expression" => {
            *complexity += 1;
        }
        "list_comprehension"
        | "set_comprehension"
        | "dictionary_comprehension"
        | "generator_expression" => {
            *complexity += 1;
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
        "if_statement"
            | "while_statement"
            | "for_statement"
            | "try_statement"
            | "with_statement"
            | "match_statement"
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

fn extract_calls_from_body(
    node: &Node,
    source: &[u8],
    file_path: &str,
    function_name: &str,
    impl_type: Option<&str>,
    result: &mut ParsedFile,
) {
    let mut callees = Vec::new();
    collect_calls(node, source, &mut callees);

    if !callees.is_empty() {
        result.call_graph.push(CallInfo::new(
            file_path.to_string(),
            function_name.to_string(),
            impl_type.map(|s| s.to_string()),
            node.start_position().row + 1,
        ));
        if let Some(call_info) = result.call_graph.last_mut() {
            call_info.callees = callees;
        }
    }
}

fn collect_calls(node: &Node, source: &[u8], callees: &mut Vec<CallEdge>) {
    if node.kind() == "call" {
        if let Some(function) = node.child_by_field_name("function") {
            let line = node.start_position().row + 1;
            let is_await = node.parent().is_some_and(|p| p.kind() == "await");

            match function.kind() {
                "identifier" => {
                    let name = node_text(&function, source);
                    callees.push(CallEdge {
                        target: name,
                        target_type: None,
                        line,
                        is_async_call: is_await,
                        is_try_call: false,
                    });
                }
                "attribute" => {
                    if let Some(attr) = function.child_by_field_name("attribute") {
                        let method_name = node_text(&attr, source);
                        let receiver = function
                            .child_by_field_name("object")
                            .map(|o| node_text(&o, source));
                        callees.push(CallEdge {
                            target: method_name,
                            target_type: receiver,
                            line,
                            is_async_call: is_await,
                            is_try_call: false,
                        });
                    }
                }
                _ => {}
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_calls(&child, source, callees);
    }
}

fn collect_await_points(node: &Node, source: &[u8], awaits: &mut Vec<AwaitPoint>) {
    if node.kind() == "await" {
        let line = node.start_position().row + 1;
        let expression = node_text(node, source);
        awaits.push(AwaitPoint { line, expression });
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_await_points(&child, source, awaits);
    }
}

fn extract_safety_from_body(
    node: &Node,
    source: &[u8],
    containing_fn: Option<&str>,
    result: &mut ParsedFile,
) {
    let line = node.start_position().row + 1;
    let text = node_text(node, source);

    match node.kind() {
        "raise_statement" => {
            let exc_type = node
                .child(1)
                .map(|c| node_text(&c, source))
                .unwrap_or_else(|| "Exception".to_string());
            result.safety.panic_points.push(PanicPoint {
                line,
                kind: PanicKind::RaiseException(exc_type),
                containing_function: containing_fn.map(|s| s.to_string()),
                context: Some(text.clone()),
            });
        }
        "assert_statement" => {
            result.safety.panic_points.push(PanicPoint {
                line,
                kind: PanicKind::AssertFalse,
                containing_function: containing_fn.map(|s| s.to_string()),
                context: Some(text.clone()),
            });
        }
        "call" => {
            check_dangerous_call(node, source, containing_fn, result);
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        extract_safety_from_body(&child, source, containing_fn, result);
    }
}

fn check_dangerous_call(
    node: &Node,
    source: &[u8],
    containing_fn: Option<&str>,
    result: &mut ParsedFile,
) {
    let call_text = node_text(node, source);
    let line = node.start_position().row + 1;

    let make_call = |category: &str, risk: RiskLevel| PythonDangerousCall {
        line,
        call_name: call_text.clone(),
        category: category.to_string(),
        containing_function: containing_fn.map(|s| s.to_string()),
        risk_level: risk,
    };

    if call_text.contains("eval(") {
        result
            .python_safety
            .dangerous_calls
            .push(make_call("eval", RiskLevel::High));
    } else if call_text.contains("exec(") {
        result
            .python_safety
            .dangerous_calls
            .push(make_call("exec", RiskLevel::High));
    } else if call_text.contains("subprocess")
        || call_text.contains("os.system")
        || call_text.contains("os.popen")
    {
        result
            .python_safety
            .dangerous_calls
            .push(make_call("subprocess", RiskLevel::High));
    } else if call_text.contains("ctypes") {
        result
            .python_safety
            .dangerous_calls
            .push(make_call("ctypes", RiskLevel::Medium));
    } else if call_text.contains("cffi") {
        result
            .python_safety
            .dangerous_calls
            .push(make_call("cffi", RiskLevel::Medium));
    } else if call_text.contains("pickle.load") || call_text.contains("pickle.loads") {
        result
            .python_safety
            .dangerous_calls
            .push(make_call("pickle", RiskLevel::High));
    } else if call_text.contains("shell=True") {
        result
            .python_safety
            .dangerous_calls
            .push(make_call("shell_injection", RiskLevel::High));
    }
}

fn extract_error_info(
    body: &Node,
    source: &[u8],
    file_path: &str,
    function_name: &str,
    impl_type: Option<&str>,
    line: usize,
    result: &mut ParsedFile,
) {
    let mut error_origins = Vec::new();
    let mut propagation_points = Vec::new();
    let mut exception_types = Vec::new();

    collect_error_patterns(
        body,
        source,
        &mut error_origins,
        &mut propagation_points,
        &mut exception_types,
    );

    if !error_origins.is_empty() || !propagation_points.is_empty() {
        let return_type = if !exception_types.is_empty() {
            ErrorReturnType::Raises { exception_types }
        } else if !error_origins.is_empty() {
            ErrorReturnType::Raises {
                exception_types: vec!["Exception".to_string()],
            }
        } else {
            ErrorReturnType::Neither
        };

        let mut error_info = ErrorInfo::new(
            file_path.to_string(),
            function_name.to_string(),
            impl_type.map(|s| s.to_string()),
            return_type,
            line,
        );
        error_info.error_origins = error_origins;
        error_info.propagation_points = propagation_points;
        result.error_info.push(error_info);
    }
}

fn collect_error_patterns(
    node: &Node,
    source: &[u8],
    origins: &mut Vec<ErrorOrigin>,
    propagations: &mut Vec<PropagationPoint>,
    exception_types: &mut Vec<String>,
) {
    match node.kind() {
        "raise_statement" => {
            let line = node.start_position().row + 1;
            let exc_type = node.child(1).map(|c| {
                let text = node_text(&c, source);
                if let Some(paren_idx) = text.find('(') {
                    text[..paren_idx].to_string()
                } else {
                    text
                }
            });

            if let Some(ref exc) = exc_type {
                if !exception_types.contains(exc) {
                    exception_types.push(exc.clone());
                }
            }

            let message = node.child(1).map(|c| {
                let text = node_text(&c, source);
                truncate_string(&text, 60)
            });

            origins.push(ErrorOrigin {
                line,
                kind: ErrorOriginKind::RaiseStatement,
                message,
            });
        }
        "assert_statement" => {
            let line = node.start_position().row + 1;
            let text = node_text(node, source);
            origins.push(ErrorOrigin {
                line,
                kind: ErrorOriginKind::AssertStatement,
                message: Some(truncate_string(&text, 60)),
            });
        }
        "try_statement" => {
            let line = node.start_position().row + 1;
            let mut has_reraise = false;
            let mut caught_exceptions = Vec::new();

            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "except_clause" {
                    if let Some(exc_type) = child.child(1) {
                        let exc_text = node_text(&exc_type, source);
                        if exc_text != ":" {
                            caught_exceptions.push(exc_text);
                        }
                    }

                    let mut inner_cursor = child.walk();
                    for inner in child.children(&mut inner_cursor) {
                        if inner.kind() == "raise_statement" && inner.child_count() == 1 {
                            has_reraise = true;
                        }
                    }
                }
            }

            let desc = if caught_exceptions.is_empty() {
                "try/except".to_string()
            } else {
                format!("try/except {}", caught_exceptions.join(", "))
            };

            propagations.push(PropagationPoint {
                line,
                expression: if has_reraise {
                    format!("{} (re-raises)", desc)
                } else {
                    desc
                },
            });
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_error_patterns(&child, source, origins, propagations, exception_types);
    }
}

fn extract_identifier_locations(root: &Node, source: &[u8], result: &mut ParsedFile) {
    collect_identifiers(root, source, &mut result.identifier_locations);
}

fn collect_identifiers(node: &Node, source: &[u8], locations: &mut Vec<(String, usize)>) {
    if node.kind() == "identifier" {
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

fn extract_test_info(root: &Node, source: &[u8], result: &mut ParsedFile) {
    collect_test_functions(root, source, &mut result.test_info);
}

fn collect_test_functions(node: &Node, source: &[u8], test_info: &mut TestInfo) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" | "decorated_definition" => {
                let (func_node, decorators) = if child.kind() == "decorated_definition" {
                    let decs = extract_decorators(&child, source);
                    let inner = child.children(&mut child.walk()).find(|c| {
                        c.kind() == "function_definition" || c.kind() == "async_function_definition"
                    });
                    match inner {
                        Some(f) => (f, decs),
                        None => continue,
                    }
                } else {
                    (child, Vec::new())
                };

                let name = func_node
                    .child_by_field_name("name")
                    .map(|n| node_text(&n, source))
                    .unwrap_or_default();

                let is_test = name.starts_with("test_")
                    || decorators.iter().any(|d| {
                        d.name.contains("pytest.mark") || d.name == "test" || d.name == "unittest"
                    });

                if is_test {
                    let line = func_node.start_position().row + 1;
                    let is_async = func_node.kind() == "async_function_definition";
                    let is_ignored = decorators
                        .iter()
                        .any(|d| d.name == "pytest.mark.skip" || d.name == "unittest.skip");
                    let should_panic = decorators.iter().any(|d| d.name == "pytest.mark.xfail");

                    let tested_function = name.strip_prefix("test_").map(|s| s.to_string());

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
            "class_definition" => {
                let class_name = child
                    .child_by_field_name("name")
                    .map(|n| node_text(&n, source))
                    .unwrap_or_default();

                if class_name.starts_with("Test")
                    || class_name.ends_with("Test")
                    || class_name.ends_with("Tests")
                {
                    let line = child.start_position().row + 1;
                    let test_count = count_test_methods(&child, source);
                    test_info.test_modules.push(TestModule {
                        name: class_name,
                        line,
                        test_count,
                    });
                }
            }
            _ => {}
        }

        collect_test_functions(&child, source, test_info);
    }
}

fn count_test_methods(node: &Node, source: &[u8]) -> usize {
    let mut count = 0;

    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            if child.kind() == "function_definition" || child.kind() == "decorated_definition" {
                let name = if child.kind() == "decorated_definition" {
                    child
                        .children(&mut child.walk())
                        .find(|c| c.kind() == "function_definition")
                        .and_then(|f| f.child_by_field_name("name"))
                        .map(|n| node_text(&n, source))
                } else {
                    child
                        .child_by_field_name("name")
                        .map(|n| node_text(&n, source))
                };

                if let Some(name) = name {
                    if name.starts_with("test_") {
                        count += 1;
                    }
                }
            }
        }
    }

    count
}

fn node_text(node: &Node, source: &[u8]) -> String {
    node.utf8_text(source).unwrap_or("").to_string()
}

fn extract_string_content(text: &str) -> String {
    let text = text.trim();

    let content = if (text.starts_with("\"\"\"") && text.ends_with("\"\"\""))
        || (text.starts_with("'''") && text.ends_with("'''"))
    {
        &text[3..text.len() - 3]
    } else if (text.starts_with('"') && text.ends_with('"'))
        || (text.starts_with('\'') && text.ends_with('\''))
    {
        &text[1..text.len() - 1]
    } else {
        text
    };

    content.trim().to_string()
}

fn is_dunder_method(name: &str) -> bool {
    name.starts_with("__") && name.ends_with("__")
}

fn truncate_string(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        return s.to_string();
    }
    let truncate_at = max_len.saturating_sub(3);
    let mut end = truncate_at;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &s[..end])
}

fn count_statements(node: &Node) -> usize {
    let mut count = 0;
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "expression_statement"
            | "return_statement"
            | "raise_statement"
            | "assert_statement"
            | "pass_statement"
            | "break_statement"
            | "continue_statement"
            | "assignment"
            | "augmented_assignment"
            | "delete_statement"
            | "import_statement"
            | "import_from_statement" => {
                count += 1;
            }
            "if_statement" | "for_statement" | "while_statement" | "try_statement"
            | "with_statement" | "match_statement" => {
                count += 1;
                count += count_statements(&child);
            }
            _ => {
                count += count_statements(&child);
            }
        }
    }

    count
}

fn collect_early_returns(node: &Node, source: &[u8]) -> Vec<String> {
    let mut returns = Vec::new();
    collect_early_returns_recursive(node, source, &mut returns);
    returns.truncate(5);
    returns
}

fn collect_early_returns_recursive(node: &Node, source: &[u8], returns: &mut Vec<String>) {
    if node.kind() == "return_statement" {
        let text = node_text(node, source);
        returns.push(truncate_string(&text, 80));
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "function_definition" || child.kind() == "async_function_definition" {
            continue;
        }
        collect_early_returns_recursive(&child, source, returns);
    }
}

fn collect_key_calls(node: &Node, source: &[u8]) -> Vec<String> {
    let mut calls = Vec::new();
    collect_key_calls_recursive(node, source, &mut calls);
    calls.sort();
    calls.dedup();
    calls.truncate(10);
    calls
}

fn collect_key_calls_recursive(node: &Node, source: &[u8], calls: &mut Vec<String>) {
    if node.kind() == "call" {
        if let Some(function) = node.child_by_field_name("function") {
            let name = match function.kind() {
                "identifier" => node_text(&function, source),
                "attribute" => {
                    if let Some(attr) = function.child_by_field_name("attribute") {
                        node_text(&attr, source)
                    } else {
                        return;
                    }
                }
                _ => return,
            };
            if !name.starts_with('_') && name.chars().next().is_some_and(|c| c.is_lowercase()) {
                calls.push(name);
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "function_definition" || child.kind() == "async_function_definition" {
            continue;
        }
        collect_key_calls_recursive(&child, source, calls);
    }
}
