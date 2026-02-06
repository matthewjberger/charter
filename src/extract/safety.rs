use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SafetyInfo {
    pub unsafe_blocks: Vec<UnsafeBlock>,
    pub panic_points: Vec<PanicPoint>,
    pub unsafe_traits: Vec<String>,
    pub unsafe_impls: Vec<UnsafeImpl>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnsafeBlock {
    pub line: usize,
    pub containing_function: Option<String>,
    pub operations: Vec<UnsafeOperation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UnsafeOperation {
    RawPointerDeref,
    UnsafeFunctionCall(String),
    MutableStaticAccess(String),
    UnionFieldAccess,
    InlineAssembly,
    ExternCall(String),
    Other(String),
}

impl std::fmt::Display for UnsafeOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UnsafeOperation::RawPointerDeref => write!(f, "raw pointer deref"),
            UnsafeOperation::UnsafeFunctionCall(name) => write!(f, "unsafe call: {}", name),
            UnsafeOperation::MutableStaticAccess(name) => write!(f, "mutable static: {}", name),
            UnsafeOperation::UnionFieldAccess => write!(f, "union field access"),
            UnsafeOperation::InlineAssembly => write!(f, "inline assembly"),
            UnsafeOperation::ExternCall(name) => write!(f, "extern call: {}", name),
            UnsafeOperation::Other(desc) => write!(f, "{}", desc),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnsafeImpl {
    pub trait_name: String,
    pub type_name: String,
    pub line: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PanicPoint {
    pub line: usize,
    pub kind: PanicKind,
    pub containing_function: Option<String>,
    pub context: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PanicKind {
    Unwrap,
    Expect(String),
    PanicMacro,
    UnreachableMacro,
    TodoMacro,
    UnimplementedMacro,
    Assert,
    IndexAccess,
    RaiseException(String),
    AssertFalse,
}

impl std::fmt::Display for PanicKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PanicKind::Unwrap => write!(f, ".unwrap()"),
            PanicKind::Expect(msg) => write!(f, ".expect(\"{}\")", msg),
            PanicKind::PanicMacro => write!(f, "panic!()"),
            PanicKind::UnreachableMacro => write!(f, "unreachable!()"),
            PanicKind::TodoMacro => write!(f, "todo!()"),
            PanicKind::UnimplementedMacro => write!(f, "unimplemented!()"),
            PanicKind::Assert => write!(f, "assert!()"),
            PanicKind::IndexAccess => write!(f, "index access"),
            PanicKind::RaiseException(exc) => write!(f, "raise {}", exc),
            PanicKind::AssertFalse => write!(f, "assert False"),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LifetimeInfo {
    pub function_lifetimes: Vec<FunctionLifetime>,
    pub struct_lifetimes: Vec<StructLifetime>,
    pub complex_bounds: Vec<ComplexBound>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionLifetime {
    pub function_name: String,
    pub impl_type: Option<String>,
    pub line: usize,
    pub lifetimes: Vec<String>,
    pub has_static: bool,
    pub borrows: Vec<BorrowInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructLifetime {
    pub type_name: String,
    pub line: usize,
    pub lifetimes: Vec<String>,
    pub has_static: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BorrowInfo {
    pub param_name: String,
    pub is_mutable: bool,
    pub lifetime: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplexBound {
    pub item_name: String,
    pub line: usize,
    pub bounds: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AsyncInfo {
    pub async_functions: Vec<AsyncFunction>,
    pub blocking_calls: Vec<BlockingCall>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsyncFunction {
    pub name: String,
    pub impl_type: Option<String>,
    pub line: usize,
    pub awaits: Vec<AwaitPoint>,
    pub spawns: Vec<SpawnPoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AwaitPoint {
    pub line: usize,
    pub expression: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnPoint {
    pub line: usize,
    pub spawn_type: SpawnType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SpawnType {
    TokioSpawn,
    TokioSpawnBlocking,
    AsyncStdSpawn,
    Other(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockingCall {
    pub line: usize,
    pub call: String,
    pub in_async_context: bool,
    pub containing_function: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FeatureFlagInfo {
    pub feature_gates: Vec<FeatureGate>,
    pub cfg_blocks: Vec<CfgBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureGate {
    pub feature_name: String,
    pub symbols: Vec<GatedSymbol>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatedSymbol {
    pub name: String,
    pub kind: String,
    pub line: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CfgBlock {
    pub condition: String,
    pub line: usize,
    pub affected_items: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DocInfo {
    pub item_docs: Vec<ItemDoc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemDoc {
    pub item_name: String,
    pub item_kind: String,
    pub line: usize,
    pub summary: String,
    pub has_examples: bool,
    pub has_panics_section: bool,
    pub has_safety_section: bool,
    pub has_errors_section: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GenericConstraints {
    pub constraints: Vec<ItemConstraints>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemConstraints {
    pub item_name: String,
    pub item_kind: String,
    pub line: usize,
    pub type_params: Vec<TypeParam>,
    pub where_clause: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeParam {
    pub name: String,
    pub bounds: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TestInfo {
    pub test_functions: Vec<TestFunction>,
    pub test_modules: Vec<TestModule>,
    pub tested_items: Vec<TestedItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestFunction {
    pub name: String,
    pub line: usize,
    pub is_async: bool,
    pub is_ignored: bool,
    pub should_panic: bool,
    pub tested_function: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestModule {
    pub name: String,
    pub line: usize,
    pub test_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestedItem {
    pub item_name: String,
    pub test_names: Vec<String>,
    pub coverage_hints: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PythonSafetyInfo {
    pub dangerous_calls: Vec<PythonDangerousCall>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PythonDangerousCall {
    pub line: usize,
    pub call_name: String,
    pub category: String,
    pub containing_function: Option<String>,
    pub risk_level: RiskLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RiskLevel {
    High,
    Medium,
    Low,
}

impl std::fmt::Display for RiskLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RiskLevel::High => write!(f, "high"),
            RiskLevel::Medium => write!(f, "medium"),
            RiskLevel::Low => write!(f, "low"),
        }
    }
}
