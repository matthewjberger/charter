use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileSymbols {
    pub symbols: Vec<Symbol>,
    pub impl_map: Vec<(String, String)>,
    pub inherent_impls: Vec<InherentImpl>,
    pub macros: Vec<MacroInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InherentImpl {
    pub type_name: String,
    pub generics: String,
    pub where_clause: Option<String>,
    pub methods: Vec<ImplMethod>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImplMethod {
    pub name: String,
    pub visibility: Visibility,
    pub signature: String,
    pub is_async: bool,
    pub is_unsafe: bool,
    pub is_const: bool,
    pub line: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MacroInfo {
    pub name: String,
    pub is_exported: bool,
    pub line: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub visibility: Visibility,
    pub generics: String,
    pub line: usize,
    pub is_async: bool,
    pub is_unsafe: bool,
    pub is_const: bool,
    pub re_exported_as: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SymbolKind {
    Struct {
        fields: Vec<StructField>,
    },
    Enum {
        variants: Vec<EnumVariant>,
    },
    Trait {
        supertraits: Vec<String>,
        methods: Vec<TraitMethod>,
        associated_types: Vec<AssociatedType>,
    },
    Function {
        signature: String,
    },
    Const {
        const_type: String,
        value: Option<String>,
    },
    Static {
        static_type: String,
        is_mutable: bool,
        value: Option<String>,
    },
    TypeAlias {
        aliased_type: String,
    },
    Mod,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructField {
    pub name: String,
    pub field_type: String,
    pub visibility: Visibility,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnumVariant {
    pub name: String,
    pub payload: Option<VariantPayload>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VariantPayload {
    Tuple(Vec<String>),
    Struct(Vec<(String, String)>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraitMethod {
    pub name: String,
    pub signature: String,
    pub has_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssociatedType {
    pub name: String,
    pub bounds: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Visibility {
    Public,
    PubCrate,
    PubSuper,
    PubIn(String),
    Private,
}

impl std::fmt::Display for Visibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Visibility::Public => write!(f, "pub"),
            Visibility::PubCrate => write!(f, "pub(crate)"),
            Visibility::PubSuper => write!(f, "pub(super)"),
            Visibility::PubIn(path) => write!(f, "{}", path),
            Visibility::Private => Ok(()),
        }
    }
}

impl Visibility {
    pub fn prefix(&self) -> &str {
        match self {
            Visibility::Public => "pub ",
            Visibility::PubCrate => "pub(crate) ",
            Visibility::PubSuper => "pub(super) ",
            Visibility::PubIn(_) => "pub(in ...) ",
            Visibility::Private => "",
        }
    }
}
