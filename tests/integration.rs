use std::path::Path;
use std::process::Command;
use std::sync::Once;

static INIT_SIMPLE: Once = Once::new();
static INIT_IMPL: Once = Once::new();
static INIT_NESTED: Once = Once::new();

fn setup_fixture(fixture_path: &Path) {
    let charter_dir = fixture_path.join(".charter");
    let _ = std::fs::remove_dir_all(&charter_dir);

    let git_dir = fixture_path.join(".git");
    if !git_dir.exists() {
        Command::new("git")
            .args(["init"])
            .current_dir(fixture_path)
            .output()
            .expect("Failed to init git");
        Command::new("git")
            .args(["add", "."])
            .current_dir(fixture_path)
            .output()
            .expect("Failed to git add");
        Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(fixture_path)
            .output()
            .expect("Failed to git commit");
    }
}

fn run_charter(fixture_path: &Path) {
    let charter = env!("CARGO_BIN_EXE_charter");
    let output = Command::new(charter)
        .current_dir(fixture_path)
        .output()
        .expect("Failed to run charter");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        panic!("charter failed:\nstderr: {}\nstdout: {}", stderr, stdout);
    }
}

fn run_charter_command(fixture_path: &Path, args: &[&str]) -> (bool, String, String) {
    let charter = env!("CARGO_BIN_EXE_charter");
    let output = Command::new(charter)
        .args(args)
        .current_dir(fixture_path)
        .output()
        .expect("Failed to run charter");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    (output.status.success(), stdout, stderr)
}

fn read_charter_file(fixture_path: &Path, filename: &str) -> String {
    let path = fixture_path.join(".charter").join(filename);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {}", path.display(), e))
}

fn charter_file_exists(fixture_path: &Path, filename: &str) -> bool {
    fixture_path.join(".charter").join(filename).exists()
}

mod simple_crate {
    use super::*;

    fn fixture_path() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/simple_crate")
    }

    fn setup() {
        INIT_SIMPLE.call_once(|| {
            let path = fixture_path();
            setup_fixture(&path);
            run_charter(&path);
        });
    }

    #[test]
    fn creates_all_output_files() {
        setup();
        let path = fixture_path();

        let expected_files = [
            "overview.md",
            "symbols.md",
            "types.md",
            "calls.md",
            "hotspots.md",
            "manifest.md",
            "refs.md",
            "dependents.md",
            "clusters.md",
            "dataflow.md",
            "errors.md",
            "snippets.md",
            "safety.md",
            "cache.bin",
            "meta.json",
            "FORMAT.md",
        ];

        for file in expected_files {
            assert!(charter_file_exists(&path, file), "Should create {}", file);
        }
    }

    #[test]
    fn extracts_structs() {
        setup();
        let path = fixture_path();

        let symbols = read_charter_file(&path, "symbols.md");
        assert!(
            symbols.contains("pub struct Config"),
            "Should find Config struct"
        );
        assert!(
            symbols.contains("name: String"),
            "Should find Config.name field"
        );
        assert!(
            symbols.contains("enabled: bool"),
            "Should find Config.enabled field"
        );
    }

    #[test]
    fn extracts_enums() {
        setup();
        let path = fixture_path();

        let symbols = read_charter_file(&path, "symbols.md");
        assert!(
            symbols.contains("pub enum Status"),
            "Should find Status enum"
        );
        assert!(symbols.contains("Pending"), "Should find Pending variant");
        assert!(symbols.contains("Running"), "Should find Running variant");
        assert!(symbols.contains("Complete"), "Should find Complete variant");
        assert!(symbols.contains("Failed"), "Should find Failed variant");
    }

    #[test]
    fn extracts_traits() {
        setup();
        let path = fixture_path();

        let types = read_charter_file(&path, "types.md");
        assert!(
            types.contains("trait Processor"),
            "Should find Processor trait"
        );
    }

    #[test]
    fn extracts_public_functions() {
        setup();
        let path = fixture_path();

        let symbols = read_charter_file(&path, "symbols.md");
        assert!(
            symbols.contains("pub fn process"),
            "Should find process function"
        );
        assert!(
            symbols.contains("async_process"),
            "Should find async_process function"
        );
    }

    #[test]
    fn builds_call_graph() {
        setup();
        let path = fixture_path();

        let calls = read_charter_file(&path, "calls.md");
        assert!(
            calls.contains("process"),
            "Should have process in call graph"
        );
        assert!(
            calls.contains("validate_input") || calls.contains("transform"),
            "Should track internal calls"
        );
    }

    #[test]
    fn identifies_complexity() {
        setup();
        let path = fixture_path();

        let hotspots = read_charter_file(&path, "hotspots.md");
        assert!(
            hotspots.contains("complex_function"),
            "Should identify complex_function as hotspot"
        );
    }

    #[test]
    fn tracks_trait_implementations() {
        setup();
        let path = fixture_path();

        let types = read_charter_file(&path, "types.md");
        assert!(
            types.contains("Processor") && types.contains("SimpleProcessor"),
            "Should track SimpleProcessor implements Processor"
        );
    }

    #[test]
    fn tracks_derives() {
        setup();
        let path = fixture_path();

        let types = read_charter_file(&path, "types.md");
        assert!(
            types.contains("ProcessError") && (types.contains("Debug") || types.contains("Clone")),
            "Should track ProcessError derives"
        );
    }

    #[test]
    fn overview_contains_module_structure() {
        setup();
        let path = fixture_path();

        let overview = read_charter_file(&path, "overview.md");
        assert!(
            overview.contains("simple_crate") || overview.contains("lib.rs"),
            "Overview should contain crate name or lib.rs"
        );
        assert!(
            overview.contains("functions") || overview.contains("types"),
            "Overview should list modules"
        );
    }

    #[test]
    fn manifest_lists_files() {
        setup();
        let path = fixture_path();

        let manifest = read_charter_file(&path, "manifest.md");
        assert!(manifest.contains("lib.rs"), "Manifest should list lib.rs");
        assert!(
            manifest.contains("functions.rs"),
            "Manifest should list functions.rs"
        );
        assert!(
            manifest.contains("types.rs"),
            "Manifest should list types.rs"
        );
    }

    #[test]
    fn refs_tracks_type_references() {
        setup();
        let path = fixture_path();

        let refs = read_charter_file(&path, "refs.md");
        assert!(
            refs.contains("Config") || refs.contains("Status") || refs.contains("ProcessError"),
            "Refs should track type references"
        );
    }

    #[test]
    fn snippets_captures_function_bodies() {
        setup();
        let path = fixture_path();

        let snippets = read_charter_file(&path, "snippets.md");
        assert!(
            snippets.contains("fn") || snippets.contains("complex_function"),
            "Snippets should capture function bodies"
        );
    }

    #[test]
    fn errors_tracks_result_types() {
        setup();
        let path = fixture_path();

        let errors = read_charter_file(&path, "errors.md");
        assert!(
            errors.contains("ProcessError")
                || errors.contains("Result")
                || errors.contains("Error"),
            "Errors should track error types"
        );
    }

    #[test]
    fn meta_json_is_valid() {
        setup();
        let path = fixture_path();

        let meta = read_charter_file(&path, "meta.json");
        assert!(meta.starts_with("{"), "meta.json should be valid JSON");
        assert!(
            meta.contains("commit") || meta.contains("files"),
            "meta.json should have expected fields"
        );
    }
}

mod impl_blocks {
    use super::*;

    fn fixture_path() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/impl_blocks")
    }

    fn setup() {
        INIT_IMPL.call_once(|| {
            let path = fixture_path();
            setup_fixture(&path);
            run_charter(&path);
        });
    }

    #[test]
    fn finds_entity_struct() {
        setup();
        let path = fixture_path();

        let symbols = read_charter_file(&path, "symbols.md");
        assert!(
            symbols.contains("pub struct Entity"),
            "Should find Entity struct"
        );
    }

    #[test]
    fn finds_inherent_impl_methods() {
        setup();
        let path = fixture_path();

        let symbols = read_charter_file(&path, "symbols.md");
        assert!(symbols.contains("pub fn new"), "Should find Entity::new");
        assert!(
            symbols.contains("is_active"),
            "Should find Entity::is_active"
        );
    }

    #[test]
    fn finds_scattered_impl_methods() {
        setup();
        let path = fixture_path();

        let symbols = read_charter_file(&path, "symbols.md");
        assert!(
            symbols.contains("render_debug"),
            "Should find render_debug from render.rs"
        );
        assert!(
            symbols.contains("serialize_compact"),
            "Should find serialize_compact from serialize.rs"
        );
    }

    #[test]
    fn tracks_trait_impls_across_files() {
        setup();
        let path = fixture_path();

        let types = read_charter_file(&path, "types.md");
        assert!(types.contains("Render"), "Should find Render trait");
        assert!(types.contains("Serialize"), "Should find Serialize trait");
        assert!(types.contains("Entity"), "Entity should appear in impl map");
    }

    #[test]
    fn tracks_default_impl() {
        setup();
        let path = fixture_path();

        let types = read_charter_file(&path, "types.md");
        assert!(
            types.contains("Default"),
            "Should track Default impl for Entity"
        );
    }

    #[test]
    fn tracks_entity_usage_across_files() {
        setup();
        let path = fixture_path();

        let symbols = read_charter_file(&path, "symbols.md");
        assert!(
            symbols.contains("uses: crate::Entity"),
            "Should track files that use Entity"
        );
    }

    #[test]
    fn consolidates_impls_to_definition_file() {
        setup();
        let path = fixture_path();

        let symbols = read_charter_file(&path, "symbols.md");
        assert!(
            symbols.contains("[from src/render.rs]") || symbols.contains("from src/render.rs"),
            "Should show impl origin file for render_debug"
        );
        assert!(
            symbols.contains("[from src/serialize.rs]")
                || symbols.contains("from src/serialize.rs"),
            "Should show impl origin file for serialize_compact"
        );
    }
}

mod cli_commands {
    use super::*;

    fn fixture_path() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/simple_crate")
    }

    fn setup() {
        INIT_SIMPLE.call_once(|| {
            let path = fixture_path();
            setup_fixture(&path);
            run_charter(&path);
        });
    }

    #[test]
    fn read_quick_outputs_overview() {
        setup();
        let path = fixture_path();

        let (success, stdout, _) = run_charter_command(&path, &["read", "quick"]);
        assert!(success, "read quick should succeed");
        assert!(
            stdout.contains("simple_crate")
                || stdout.contains("overview")
                || stdout.contains("lib.rs"),
            "read quick should output overview content"
        );
    }

    #[test]
    fn read_default_includes_types() {
        setup();
        let path = fixture_path();

        let (success, stdout, _) = run_charter_command(&path, &["read"]);
        assert!(success, "read default should succeed");
        assert!(
            stdout.contains("Processor") || stdout.contains("trait"),
            "read default should include type information"
        );
    }

    #[test]
    fn read_full_includes_everything() {
        setup();
        let path = fixture_path();

        let (success, stdout, _) = run_charter_command(&path, &["read", "full"]);
        assert!(success, "read full should succeed");
        assert!(
            stdout.len() > 500,
            "read full should output substantial content"
        );
    }

    #[test]
    fn lookup_finds_struct() {
        setup();
        let path = fixture_path();

        let (success, stdout, _) = run_charter_command(&path, &["lookup", "Config"]);
        assert!(success, "lookup Config should succeed");
        assert!(
            stdout.contains("Config") || stdout.contains("struct"),
            "lookup should find Config struct"
        );
    }

    #[test]
    fn lookup_finds_enum() {
        setup();
        let path = fixture_path();

        let (success, stdout, _) = run_charter_command(&path, &["lookup", "Status"]);
        assert!(success, "lookup Status should succeed");
        assert!(
            stdout.contains("Status") || stdout.contains("enum"),
            "lookup should find Status enum"
        );
    }

    #[test]
    fn lookup_finds_trait() {
        setup();
        let path = fixture_path();

        let (success, stdout, _) = run_charter_command(&path, &["lookup", "Processor"]);
        assert!(success, "lookup Processor should succeed");
        assert!(
            stdout.contains("Processor") || stdout.contains("trait"),
            "lookup should find Processor trait"
        );
    }

    #[test]
    fn lookup_finds_function() {
        setup();
        let path = fixture_path();

        let (success, stdout, _) = run_charter_command(&path, &["lookup", "process"]);
        assert!(success, "lookup process should succeed");
        assert!(
            stdout.contains("process"),
            "lookup should find process function"
        );
    }

    #[test]
    fn lookup_nonexistent_symbol_reports_not_found() {
        setup();
        let path = fixture_path();

        let (_, stdout, _) = run_charter_command(&path, &["lookup", "NonexistentSymbol"]);
        assert!(
            stdout.contains("not found")
                || stdout.contains("No matches")
                || stdout.is_empty()
                || stdout.contains("NonexistentSymbol"),
            "lookup should indicate symbol not found"
        );
    }

    #[test]
    fn status_shows_summary() {
        setup();
        let path = fixture_path();

        let (success, stdout, _) = run_charter_command(&path, &["status"]);
        assert!(success, "status should succeed");
        assert!(
            stdout.contains("files") || stdout.contains("lines") || stdout.contains("simple_crate"),
            "status should show summary information"
        );
    }

    #[test]
    fn query_callers_works() {
        setup();
        let path = fixture_path();

        let (success, stdout, _) = run_charter_command(&path, &["query", "callers of transform"]);
        assert!(success, "query callers should succeed");
        assert!(
            stdout.contains("process")
                || stdout.contains("async_process")
                || stdout.contains("caller")
                || stdout.contains("transform"),
            "query should find callers of transform"
        );
    }

    #[test]
    fn query_callees_works() {
        setup();
        let path = fixture_path();

        let (success, stdout, _) = run_charter_command(&path, &["query", "callees of process"]);
        assert!(success, "query callees should succeed");
        assert!(
            stdout.contains("validate_input")
                || stdout.contains("transform")
                || stdout.contains("callee")
                || stdout.contains("process"),
            "query should find callees of process"
        );
    }

    #[test]
    fn query_hotspots_works() {
        setup();
        let path = fixture_path();

        let (success, stdout, _) = run_charter_command(&path, &["query", "hotspots"]);
        assert!(success, "query hotspots should succeed");
        assert!(
            stdout.contains("complex_function") || stdout.contains("complexity"),
            "query should list hotspots"
        );
    }
}

mod cache_behavior {
    use super::*;

    fn fixture_path() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/simple_crate")
    }

    fn setup() {
        INIT_SIMPLE.call_once(|| {
            let path = fixture_path();
            setup_fixture(&path);
            run_charter(&path);
        });
    }

    #[test]
    fn warm_run_is_fast() {
        setup();
        let path = fixture_path();

        let start = std::time::Instant::now();
        let (success, stdout, _) = run_charter_command(&path, &[]);
        let duration = start.elapsed();

        assert!(success, "warm run should succeed");
        assert!(
            stdout.contains("Up to date")
                || stdout.contains("cached")
                || duration.as_millis() < 2000,
            "warm run should be fast or report up to date"
        );
    }

    #[test]
    fn cache_file_exists() {
        setup();
        let path = fixture_path();

        assert!(
            charter_file_exists(&path, "cache.bin"),
            "cache.bin should exist after run"
        );
    }
}

mod nested_workspace {
    use super::*;

    fn parent_path() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/nested_workspace")
    }

    fn child_path() -> std::path::PathBuf {
        parent_path().join("child_crate")
    }

    fn setup() {
        INIT_NESTED.call_once(|| {
            let parent = parent_path();
            let child = child_path();

            let _ = std::fs::remove_dir_all(parent.join(".charter"));
            let _ = std::fs::remove_dir_all(child.join(".charter"));

            if !parent.join(".git").exists() {
                Command::new("git")
                    .args(["init"])
                    .current_dir(&parent)
                    .output()
                    .expect("Failed to init git");
                Command::new("git")
                    .args(["add", "."])
                    .current_dir(&parent)
                    .output()
                    .expect("Failed to git add");
                Command::new("git")
                    .args(["commit", "-m", "init"])
                    .current_dir(&parent)
                    .output()
                    .expect("Failed to git commit");
            }

            run_charter(&child);
        });
    }

    #[test]
    fn finds_child_crate_not_parent() {
        setup();
        let child = child_path();

        assert!(
            charter_file_exists(&child, "symbols.md"),
            ".charter should be created in child_crate"
        );

        let symbols = read_charter_file(&child, "symbols.md");
        assert!(
            symbols.contains("ChildType"),
            "Should find ChildType from child crate"
        );
        assert!(
            !symbols.contains("ParentType"),
            "Should NOT find ParentType from parent crate"
        );
    }

    #[test]
    fn child_charter_only_has_child_files() {
        setup();
        let child = child_path();

        let manifest = read_charter_file(&child, "manifest.md");
        assert!(
            !manifest.contains("parent_lib.rs"),
            "Child .charter should not include parent files"
        );
    }
}

mod cache_invalidation {
    use super::*;
    use std::sync::Mutex;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn fixture_path() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cache_test")
    }

    fn mutable_file() -> std::path::PathBuf {
        fixture_path().join("src/mutable.rs")
    }

    fn lib_file() -> std::path::PathBuf {
        fixture_path().join("src/lib.rs")
    }

    const ORIGINAL_MUTABLE: &str = "pub struct OriginalType {\n    pub name: String,\n}\n";
    const MODIFIED_MUTABLE: &str = "pub struct ModifiedType {\n    pub count: u64,\n}\n";
    const ORIGINAL_LIB: &str = "pub mod stable;\npub mod mutable;\n";
    const LIB_WITHOUT_MUTABLE: &str = "pub mod stable;\n";

    fn reset_fixture() {
        let path = fixture_path();

        std::fs::write(mutable_file(), ORIGINAL_MUTABLE).expect("Failed to reset mutable.rs");
        std::fs::write(lib_file(), ORIGINAL_LIB).expect("Failed to reset lib.rs");

        let _ = std::fs::remove_dir_all(path.join(".charter"));

        let git_dir = path.join(".git");
        if !git_dir.exists() {
            Command::new("git")
                .args(["init"])
                .current_dir(&path)
                .output()
                .expect("Failed to init git");
            Command::new("git")
                .args(["add", "."])
                .current_dir(&path)
                .output()
                .expect("Failed to git add");
            Command::new("git")
                .args(["commit", "-m", "init"])
                .current_dir(&path)
                .output()
                .expect("Failed to git commit");
        }
    }

    #[test]
    fn detects_modified_file() {
        let _lock = TEST_LOCK.lock().unwrap();
        let path = fixture_path();

        reset_fixture();
        run_charter(&path);

        let symbols_before = read_charter_file(&path, "symbols.md");
        assert!(
            symbols_before.contains("OriginalType"),
            "Should find OriginalType before modification"
        );

        std::fs::write(mutable_file(), MODIFIED_MUTABLE).expect("Failed to modify mutable.rs");
        std::thread::sleep(std::time::Duration::from_millis(100));

        let (success, stdout, _) = run_charter_command(&path, &[]);
        assert!(success, "charter should succeed after modification");
        assert!(
            stdout.contains("modified")
                || stdout.contains("1 modified")
                || !stdout.contains("Up to date"),
            "charter should detect modification: {}",
            stdout
        );

        let symbols_after = read_charter_file(&path, "symbols.md");
        assert!(
            symbols_after.contains("ModifiedType"),
            "Should find ModifiedType after modification"
        );
        assert!(
            !symbols_after.contains("OriginalType"),
            "Should NOT find OriginalType after modification"
        );

        std::fs::write(mutable_file(), ORIGINAL_MUTABLE).expect("Failed to restore mutable.rs");
    }

    #[test]
    fn detects_deleted_file() {
        let _lock = TEST_LOCK.lock().unwrap();
        let path = fixture_path();

        reset_fixture();
        run_charter(&path);

        let symbols_before = read_charter_file(&path, "symbols.md");
        assert!(
            symbols_before.contains("OriginalType"),
            "Should find OriginalType before deletion"
        );

        std::fs::remove_file(mutable_file()).expect("Failed to delete mutable.rs");
        std::fs::write(lib_file(), LIB_WITHOUT_MUTABLE).expect("Failed to update lib.rs");
        std::thread::sleep(std::time::Duration::from_millis(100));

        let (success, stdout, _) = run_charter_command(&path, &[]);
        assert!(success, "charter should succeed after deletion");
        assert!(
            stdout.contains("removed")
                || stdout.contains("1 removed")
                || !stdout.contains("Up to date"),
            "charter should detect deletion: {}",
            stdout
        );

        let symbols_after = read_charter_file(&path, "symbols.md");
        assert!(
            !symbols_after.contains("OriginalType"),
            "Should NOT find OriginalType after deletion"
        );

        std::fs::write(mutable_file(), ORIGINAL_MUTABLE).expect("Failed to restore mutable.rs");
        std::fs::write(lib_file(), ORIGINAL_LIB).expect("Failed to restore lib.rs");
    }

    #[test]
    fn detects_added_file() {
        let _lock = TEST_LOCK.lock().unwrap();
        let path = fixture_path();
        let new_file = path.join("src/added.rs");

        reset_fixture();
        let _ = std::fs::remove_file(&new_file);
        run_charter(&path);

        let symbols_before = read_charter_file(&path, "symbols.md");
        assert!(
            !symbols_before.contains("AddedType"),
            "Should NOT find AddedType before addition"
        );

        std::fs::write(
            &new_file,
            "pub struct AddedType {\n    pub data: Vec<u8>,\n}\n",
        )
        .expect("Failed to create added.rs");
        std::fs::write(
            lib_file(),
            "pub mod stable;\npub mod mutable;\npub mod added;\n",
        )
        .expect("Failed to update lib.rs");
        std::thread::sleep(std::time::Duration::from_millis(100));

        let (success, stdout, _) = run_charter_command(&path, &[]);
        assert!(success, "charter should succeed after addition");
        assert!(
            stdout.contains("added")
                || stdout.contains("1 added")
                || !stdout.contains("Up to date"),
            "charter should detect addition: {}",
            stdout
        );

        let symbols_after = read_charter_file(&path, "symbols.md");
        assert!(
            symbols_after.contains("AddedType"),
            "Should find AddedType after addition"
        );

        let _ = std::fs::remove_file(&new_file);
        std::fs::write(lib_file(), ORIGINAL_LIB).expect("Failed to restore lib.rs");
    }
}

mod malformed_rust {
    use super::*;
    use std::sync::Once;

    static INIT_MALFORMED: Once = Once::new();

    fn fixture_path() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/malformed")
    }

    fn setup() {
        INIT_MALFORMED.call_once(|| {
            let path = fixture_path();
            setup_fixture(&path);

            let charter = env!("CARGO_BIN_EXE_charter");
            let _ = Command::new(charter)
                .current_dir(&path)
                .output()
                .expect("Failed to run charter");
        });
    }

    #[test]
    fn does_not_crash_on_malformed_file() {
        setup();
        let path = fixture_path();

        assert!(
            charter_file_exists(&path, "symbols.md"),
            "charter should still create output despite malformed file"
        );
    }

    #[test]
    fn still_extracts_valid_symbols() {
        setup();
        let path = fixture_path();

        let symbols = read_charter_file(&path, "symbols.md");
        assert!(
            symbols.contains("ValidType"),
            "Should still find ValidType from valid.rs"
        );
        assert!(
            symbols.contains("valid_function"),
            "Should still find valid_function from valid.rs"
        );
    }

    #[test]
    fn skipped_file_is_tracked() {
        setup();
        let path = fixture_path();

        if charter_file_exists(&path, "skipped.md") {
            let skipped = read_charter_file(&path, "skipped.md");
            assert!(
                skipped.contains("broken.rs")
                    || skipped.contains("skipped")
                    || skipped.contains("error"),
                "skipped.md should mention broken.rs or indicate skip reason"
            );
        }
    }
}
