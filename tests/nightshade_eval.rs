use charter::external;
use charter::index::build_index;
use std::path::PathBuf;

fn nightshade_root() -> PathBuf {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap();
    PathBuf::from(home)
        .join("code")
        .join("nightshade")
        .join("crates")
        .join("nightshade")
}

#[tokio::test]
async fn scenario_1_search_spritepass() {
    let root = nightshade_root();
    if !root.exists() {
        return;
    }
    let index = build_index(&root).await.unwrap();

    let matches: Vec<_> = index
        .symbols_by_name
        .iter()
        .filter(|(name, _)| name.to_lowercase().contains("spritepass"))
        .flat_map(|(name, syms)| {
            syms.iter()
                .map(move |s| format!("{} [{}] {}:{}", name, s.kind, s.file, s.line))
        })
        .collect();

    println!("\n=== SCENARIO 1: Search SpritePass ===");
    for m in &matches {
        println!("  {}", m);
    }
    assert!(!matches.is_empty(), "SpritePass should exist in nightshade");
}

#[tokio::test]
async fn scenario_2_self_resolution() {
    let root = nightshade_root();
    if !root.exists() {
        return;
    }
    let index = build_index(&root).await.unwrap();

    let upstream: Vec<_> = index
        .call_graph
        .iter()
        .filter(|(k, _)| k.contains("SpritePass::"))
        .flat_map(|(caller, targets)| {
            targets
                .iter()
                .map(move |t| (caller.clone(), t.name.clone(), t.receiver_type.clone()))
        })
        .collect();

    println!("\n=== SCENARIO 2: Self:: resolution ===");
    let mut self_count = 0;
    let mut resolved_count = 0;
    for (caller, target, receiver) in &upstream {
        if target.contains("Self::") || receiver.as_deref() == Some("Self") {
            self_count += 1;
            println!("  FAIL Self::: {} -> {} (receiver={:?})", caller, target, receiver);
        }
        if target.contains("SpritePass::") {
            resolved_count += 1;
        }
    }
    println!("  Self:: remaining: {} (should be 0)", self_count);
    println!("  SpritePass:: resolved: {}", resolved_count);
    for (caller, target, _) in upstream.iter().take(10) {
        println!("  {} -> {}", caller, target);
    }
    assert_eq!(self_count, 0, "No Self:: references should remain");
}

#[tokio::test]
async fn scenario_3_snippet_hints() {
    let root = nightshade_root();
    if !root.exists() {
        return;
    }
    let index = build_index(&root).await.unwrap();

    println!("\n=== SCENARIO 3: Snippet hints ===");
    let mut hint_count = 0;
    let mut full_count = 0;
    for snippets in index.snippets_by_name.values() {
        for snip in snippets {
            if snip.hint.is_some() {
                hint_count += 1;
            } else {
                full_count += 1;
            }
        }
    }
    println!("  Snippets with hint (summary): {}", hint_count);
    println!("  Snippets with full body: {}", full_count);

    let examples: Vec<_> = index
        .snippets_by_name
        .iter()
        .filter(|(name, _)| name.contains("SpritePass::") || name.contains("MeshPass::"))
        .flat_map(|(name, snips)| snips.iter().map(move |s| (name.clone(), s.clone())))
        .take(5)
        .collect();

    for (name, snip) in &examples {
        let body_preview = if snip.body.len() > 100 {
            format!("{}...", &snip.body[..100])
        } else {
            snip.body.clone()
        };
        println!(
            "  {} @ {}:{}-{} imp={} hint={} body={}",
            name,
            snip.file,
            snip.line,
            snip.end_line,
            snip.importance_score,
            snip.hint.as_deref().unwrap_or("(none)"),
            body_preview,
        );
    }

    assert!(hint_count > 0, "Should have some summary-only snippets with hints");
}

#[tokio::test]
async fn scenario_4_imports() {
    let root = nightshade_root();
    if !root.exists() {
        return;
    }
    let index = build_index(&root).await.unwrap();

    println!("\n=== SCENARIO 4: Import tracking ===");

    let sprite_file = "crates/nightshade/src/render/wgpu/passes/geometry/sprite.rs";
    let file_imports = index.imports_by_file.get(sprite_file);
    if let Some(imports) = file_imports {
        println!("  Imports in sprite.rs: {}", imports.len());
        for imp in imports.iter().take(10) {
            println!("    L{}: {} -> {:?}", imp.line, imp.path, imp.symbols);
        }
    } else {
        let partial: Vec<_> = index
            .imports_by_file
            .keys()
            .filter(|k| k.contains("sprite.rs") && k.contains("passes"))
            .collect();
        println!("  sprite.rs not found by exact path. Partial matches: {:?}", partial);
        if let Some(key) = partial.first() {
            let imports = &index.imports_by_file[*key];
            println!("  Imports in {}: {}", key, imports.len());
            for imp in imports.iter().take(10) {
                println!("    L{}: {} -> {:?}", imp.line, imp.path, imp.symbols);
            }
        }
    }

    let camera_locs = index.imports_by_symbol.get("Camera");
    if let Some(locs) = camera_locs {
        println!("\n  Files importing 'Camera': {}", locs.len());
        for loc in locs.iter().take(10) {
            println!("    {}:{} from {}", loc.file, loc.line, loc.source_path);
        }
    } else {
        println!("\n  No files importing 'Camera' directly");
    }

    assert!(!index.imports_by_file.is_empty(), "Should have file imports");
    assert!(!index.imports_by_symbol.is_empty(), "Should have symbol imports");
}

#[tokio::test]
async fn scenario_5_summary() {
    let root = nightshade_root();
    if !root.exists() {
        return;
    }
    let index = build_index(&root).await.unwrap();

    println!("\n=== SCENARIO 5: Codebase summary ===");
    println!("  Files: {}", index.result.files.len());
    println!(
        "  Total lines: {}",
        index.result.files.iter().map(|f| f.lines).sum::<usize>()
    );
    println!("  Symbols indexed: {}", index.symbols_by_name.len());
    println!("  Call graph entries: {}", index.call_graph.len());
    println!("  Reverse calls entries: {}", index.reverse_calls.len());
    println!("  Impl map entries: {}", index.impl_map.len());
    println!("  Derive map entries: {}", index.derive_map.len());
    println!("  Snippets: {}", index.snippets_by_name.len());
    println!("  Import files tracked: {}", index.imports_by_file.len());
    println!("  Import symbols tracked: {}", index.imports_by_symbol.len());
}

#[tokio::test]
async fn scenario_6_callers() {
    let root = nightshade_root();
    if !root.exists() {
        return;
    }
    let index = build_index(&root).await.unwrap();

    println!("\n=== SCENARIO 6: Callers of SpritePass::prepare ===");
    let callers = index.reverse_calls.get("SpritePass::prepare");
    if let Some(callers) = callers {
        for c in callers {
            println!(
                "  {} (impl_type={:?}) @ {}:{}",
                c.name, c.impl_type, c.file, c.line
            );
        }
    } else {
        println!("  No direct callers found for SpritePass::prepare");
        let partial: Vec<_> = index
            .reverse_calls
            .keys()
            .filter(|k| k.contains("prepare") && k.contains("Sprite"))
            .collect();
        println!("  Partial matches: {:?}", partial);
    }
}

#[tokio::test]
async fn scenario_7_type_hierarchy() {
    let root = nightshade_root();
    if !root.exists() {
        return;
    }
    let index = build_index(&root).await.unwrap();

    println!("\n=== SCENARIO 7: Renderer type hierarchy ===");
    if let Some(impls) = index.impl_map.get("Renderer") {
        println!("  Implementors of Renderer trait:");
        for imp in impls {
            println!("    {} @ {}:{}", imp.type_name, imp.file, imp.line);
        }
    }
    if let Some(traits) = index.reverse_impl_map.get("WgpuRenderer") {
        println!("  WgpuRenderer implements:");
        for t in traits {
            println!("    {}", t);
        }
    }
}

#[tokio::test]
async fn scenario_8_definition() {
    let root = nightshade_root();
    if !root.exists() {
        return;
    }
    let index = build_index(&root).await.unwrap();

    println!("\n=== SCENARIO 8: Camera definition ===");
    if let Some(syms) = index.symbols_by_name.get("Camera") {
        for s in syms {
            println!("  {} [{}] {}:{} vis={}", s.name, s.kind, s.file, s.line, s.visibility);
        }
    }
    if let Some((file, line)) = index.symbol_table.get("Camera") {
        println!("  Symbol table: {}:{}", file, line);
    }
}

#[tokio::test]
async fn scenario_9_external_crates() {
    let root = nightshade_root();
    if !root.exists() {
        return;
    }

    println!("\n=== SCENARIO 9: External crate indexing ===");
    let deps = external::parse_direct_deps(&root);
    println!("  Direct deps: {}", deps.len());
    for dep in deps.iter().take(15) {
        println!("    {}", dep);
    }

    let crates = external::collect_external_crates(&root, &deps);
    println!("\n  Located crates: {}", crates.len());
    for c in crates.iter().take(10) {
        println!("    {} v{} @ {}", c.name, c.version, c.source_dir.display());
    }

    let symbols = external::extract_external_symbols(&crates);
    println!("\n  External symbols extracted: {}", symbols.len());

    let device_syms: Vec<_> = symbols
        .iter()
        .filter(|s| s.name.contains("Device"))
        .collect();
    println!("  Symbols matching 'Device': {}", device_syms.len());
    for s in device_syms.iter().take(5) {
        println!(
            "    {} [{}] from {} @ {}:{}",
            s.name, s.kind, s.crate_name, s.file, s.line
        );
        if let Some(ref sig) = s.signature {
            println!("      sig: {}", &sig[..sig.len().min(100)]);
        }
    }

    let render_pass: Vec<_> = symbols
        .iter()
        .filter(|s| s.name == "RenderPass" || s.name.ends_with("::RenderPass"))
        .collect();
    println!("\n  Symbols matching 'RenderPass': {}", render_pass.len());
    for s in &render_pass {
        println!(
            "    {} [{}] from {} @ {}:{}",
            s.name, s.kind, s.crate_name, s.file, s.line
        );
    }
}
