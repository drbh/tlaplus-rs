//! Typed surface syntax with explicit transition structure and source locations.
use crate::Location;

#[derive(Clone, Debug)]
pub struct Name {
    pub text: String,
    pub at: Location,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    Int,
    Bool,
    String,
    Enum(String),
    Refined(Box<Type>, String),
    Record(String, Vec<(String, Type)>),
    Set(Box<Type>),
    Seq(Box<Type>),
    Map(Box<Type>, Box<Type>),
    Function(Vec<Type>, Box<Type>),
    /// Element type of an empty collection, unified at its use site.
    Unknown,
    Tuple(Vec<Type>),
}
#[derive(Clone, Debug)]
pub struct Expr {
    pub at: Location,
    pub kind: Kind,
}
#[derive(Clone, Debug)]
pub enum Kind {
    Int(String),
    String(String),
    Bool(bool),
    Name(String),
    Unary(String, Box<Expr>),
    Binary(String, Box<Expr>, Box<Expr>),
    Collection(bool, Vec<Expr>),
    Tuple(Vec<Expr>),
    Index(Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    Binder(String, Name, Box<Expr>, Box<Expr>),
    Lambda(Name, Type, Box<Expr>),
    TypeTest(Type, Box<Expr>),
    TotalTest(Type, Box<Expr>),
    Record(String, Vec<(Name, Expr)>),
    Field(Box<Expr>, Name),
}
#[derive(Clone, Debug)]
pub struct Binding {
    pub name: Name,
    pub ty: Type,
    pub value: Expr,
    pub choice: bool,
    pub initialized: bool,
}
#[derive(Clone, Debug)]
pub struct Enumeration {
    pub name: Name,
    pub variants: Vec<Name>,
}
#[derive(Clone, Debug)]
pub struct Refinement {
    pub name: Name,
    pub base: Type,
    pub binding: Name,
    pub predicate: Expr,
}
#[derive(Clone, Debug)]
pub struct Function {
    pub name: Name,
    pub params: Vec<(Name, Type)>,
    pub result: Type,
    pub body: Expr,
    pub recursive: bool,
    pub stateful: bool,
}
#[derive(Clone, Debug)]
pub struct Action {
    pub name: Name,
    pub params: Vec<(Name, Expr)>,
    pub picks: Vec<(Name, Expr)>,
    pub guards: Vec<Expr>,
    pub updates: Vec<(Name, Expr)>,
    pub calls: Vec<(Name, Vec<Expr>)>,
}
#[derive(Clone, Debug)]
pub struct Property {
    pub name: Name,
    pub body: Expr,
    pub temporal: bool,
    pub frame: Vec<Name>,
}
#[derive(Clone, Debug)]
pub struct Fairness {
    pub strong: bool,
    pub actions: Vec<Name>,
}
#[derive(Clone, Debug)]
pub struct Spec {
    pub name: Name,
    pub library: bool,
    pub imports: Vec<Name>,
    pub enums: Vec<Enumeration>,
    pub records: Vec<(Name, Vec<(Name, Type)>)>,
    pub refinements: Vec<Refinement>,
    pub domains: Vec<(Name, Expr)>,
    pub definition_order: Vec<(bool, usize)>,
    pub init: Option<Action>,
    pub next: Option<Vec<Name>>,
    pub refinement_checks: Vec<Expr>,
    pub constants: Vec<Binding>,
    pub states: Vec<Binding>,
    pub functions: Vec<Function>,
    pub assumptions: Vec<Expr>,
    pub actions: Vec<Action>,
    pub properties: Vec<Property>,
    pub fairness: Vec<Fairness>,
}
