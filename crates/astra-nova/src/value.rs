use std::cell::RefCell;
use std::fmt;
use std::fmt::Write as _;
use std::rc::Rc;

use astra_core::ast;
use astra_core::types::Type;

use crate::error::Panic;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Integer {
    pub value: i128,
    pub ty: Type,
}

impl Integer {
    #[must_use]
    pub const fn new(value: i128, ty: Type) -> Self {
        Self { value, ty }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Float {
    pub value: f64,
    pub ty: Type,
}

impl Float {
    #[must_use]
    pub fn new(value: f64, ty: Type) -> Self {
        if ty == Type::F32 {
            Self {
                value: f64::from(value as f32),
                ty,
            }
        } else {
            Self { value, ty }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum StructFieldsValue {
    Named(Vec<(String, Value)>),
    Tuple(Vec<Value>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructValue {
    pub name: String,
    pub fields: StructFieldsValue,
}

impl StructValue {
    #[must_use]
    pub fn field(&self, name: &str) -> Option<&Value> {
        match &self.fields {
            StructFieldsValue::Named(fields) => fields
                .iter()
                .find_map(|(field, value)| (field == name).then_some(value)),
            StructFieldsValue::Tuple(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnumValue {
    pub enum_name: String,
    pub variant: String,
    pub fields: Vec<Value>,
}

#[derive(Debug)]
pub struct FunctionValue {

    pub(crate) module: String,
    pub(crate) declaration: Rc<ast::FunctionDecl>,
}

impl FunctionValue {
    pub(crate) fn new(module: impl Into<String>, declaration: Rc<ast::FunctionDecl>) -> Self {
        Self {
            module: module.into(),
            declaration,
        }
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.declaration.name
    }

    #[must_use]
    pub fn module(&self) -> &str {
        &self.module
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinFunction {
    Print,
    Println,
    Dbg,
    Assert,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ConstructorKind {
    Some,
    Ok,
    Err,
    BytesNew,
    ChannelCreate,
    TupleStruct {
        module: String,
        name: String,
    },
    EnumVariant {
        module: String,
        enum_name: String,
        variant: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Constructor {
    pub(crate) kind: ConstructorKind,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum TaskFailure {
    Error(Value),
    Panic(Panic),
}

#[derive(Debug)]
pub struct Task {
    pub(crate) outcome: Option<Result<Value, TaskFailure>>,
    pub(crate) cancelled: bool,
    pub(crate) joined: bool,
}

impl Task {
    pub(crate) fn complete(outcome: Result<Value, TaskFailure>) -> Self {
        Self {
            outcome: Some(outcome),
            cancelled: false,
            joined: false,
        }
    }
}

#[derive(Debug)]
pub struct ChannelValue {
    pub(crate) buffer: Vec<Value>,
    pub(crate) capacity: usize,
    pub(crate) closed: bool,
}

impl ChannelValue {
    pub(crate) fn new() -> Self {
        Self {
            buffer: Vec::new(),
            capacity: 16,
            closed: false,
        }
    }
}

#[derive(Clone)]
pub enum Value {
    Unit,
    Bool(bool),
    Integer(Integer),
    Float(Float),
    Str(Rc<str>),
    Bytes(Rc<RefCell<Vec<u8>>>),
    Array(Rc<RefCell<Vec<Value>>>),

    Map(Rc<RefCell<Vec<(Value, Value)>>>),
    Tuple(Rc<RefCell<Vec<Value>>>),
    Struct(Rc<RefCell<StructValue>>),
    Enum(Rc<RefCell<EnumValue>>),
    Option(Option<Box<Value>>),
    Result(Result<Box<Value>, Box<Value>>),
    Function(Rc<FunctionValue>),
    Builtin(BuiltinFunction),
    BoundMethod {
        receiver: Box<Value>,
        name: String,
    },
    Constructor(Constructor),
    Task(Rc<RefCell<Task>>),
    Channel(Rc<RefCell<ChannelValue>>),
    ForeignModule(String),
    ScriptBlock {
        name: String,
        language: String,
        span: astra_core::span::Span,
    },
    ScriptFunction {
        script_name: String,
        function_name: String,
        language: String,
        span: astra_core::span::Span,
    },

    ModuleNamespace(String),

    EnumNamespace {
        module: String,
        enum_name: String,
    },
    Imported {
        module: String,
        name: String,
    },

    Failure(Box<Value>),
}

impl Value {

    #[must_use]
    pub fn string(value: impl Into<String>) -> Self {
        Self::Str(Rc::from(value.into()))
    }

    #[must_use]
    pub fn array(values: Vec<Value>) -> Self {
        Self::Array(Rc::new(RefCell::new(values)))
    }

    #[must_use]
    pub fn map(values: Vec<(Value, Value)>) -> Self {
        Self::Map(Rc::new(RefCell::new(values)))
    }

    #[must_use]
    pub fn named_struct(name: impl Into<String>, fields: Vec<(String, Value)>) -> Self {
        Self::Struct(Rc::new(RefCell::new(StructValue {
            name: name.into(),
            fields: StructFieldsValue::Named(fields),
        })))
    }

    #[must_use]
    pub fn tuple_struct(name: impl Into<String>, fields: Vec<Value>) -> Self {
        Self::Struct(Rc::new(RefCell::new(StructValue {
            name: name.into(),
            fields: StructFieldsValue::Tuple(fields),
        })))
    }

    #[must_use]
    pub fn enum_value(
        enum_name: impl Into<String>,
        variant: impl Into<String>,
        fields: Vec<Value>,
    ) -> Self {
        Self::Enum(Rc::new(RefCell::new(EnumValue {
            enum_name: enum_name.into(),
            variant: variant.into(),
            fields,
        })))
    }

    #[must_use]
    pub const fn as_integer(&self) -> Option<i128> {
        match self {
            Self::Integer(integer) => Some(integer.value),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(value) => Some(value),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => None,
        }
    }

    #[must_use]
    pub fn render(&self) -> String {
        let mut output = String::new();
        self.write(false, &mut output);
        output
    }

    #[must_use]
    pub fn render_debug(&self) -> String {
        let mut output = String::new();
        self.write(true, &mut output);
        output
    }

    fn write(&self, debug: bool, output: &mut String) {
        match self {
            Self::Unit => output.push_str("()"),
            Self::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
            Self::Integer(integer) => output.push_str(&integer.value.to_string()),
            Self::Float(float) => write_float(float.value, output),
            Self::Str(value) => {
                if debug {
                    write!(output, "{value:?}").expect("writing to a String cannot fail");
                } else {
                    output.push_str(value);
                }
            }
            Self::Bytes(bytes) => write_bytes(&bytes.borrow(), output),
            Self::Array(values) => {
                output.push('[');
                for (index, value) in values.borrow().iter().enumerate() {
                    if index > 0 {
                        output.push_str(", ");
                    }
                    value.write(debug, output);
                }
                output.push(']');
            }
            Self::Map(entries) => {
                output.push('{');
                for (index, (key, value)) in entries.borrow().iter().enumerate() {
                    if index > 0 {
                        output.push_str(", ");
                    }
                    key.write(debug, output);
                    output.push_str(": ");
                    value.write(debug, output);
                }
                output.push('}');
            }
            Self::Tuple(values) => {
                let values = values.borrow();
                output.push('(');
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        output.push_str(", ");
                    }
                    value.write(debug, output);
                }
                if values.len() == 1 {
                    output.push(',');
                }
                output.push(')');
            }
            Self::Struct(value) => {
                let value = value.borrow();
                output.push_str(&value.name);
                match &value.fields {
                    StructFieldsValue::Named(fields) => {
                        output.push_str(" { ");
                        for (index, (name, field)) in fields.iter().enumerate() {
                            if index > 0 {
                                output.push_str(", ");
                            }
                            output.push_str(name);
                            output.push_str(": ");
                            field.write(debug, output);
                        }
                        output.push_str(" }");
                    }
                    StructFieldsValue::Tuple(fields) => {
                        output.push('(');
                        for (index, field) in fields.iter().enumerate() {
                            if index > 0 {
                                output.push_str(", ");
                            }
                            field.write(debug, output);
                        }
                        output.push(')');
                    }
                }
            }
            Self::Enum(value) => {
                let value = value.borrow();
                output.push_str(&value.enum_name);
                output.push('.');
                output.push_str(&value.variant);
                if !value.fields.is_empty() {
                    output.push('(');
                    for (index, field) in value.fields.iter().enumerate() {
                        if index > 0 {
                            output.push_str(", ");
                        }
                        field.write(debug, output);
                    }
                    output.push(')');
                }
            }
            Self::Option(value) => match value {
                Some(value) => {
                    output.push_str("Some(");
                    value.write(debug, output);
                    output.push(')');
                }
                None => output.push_str("None"),
            },
            Self::Result(value) => match value {
                Ok(value) => {
                    output.push_str("Ok(");
                    value.write(debug, output);
                    output.push(')');
                }
                Err(value) => {
                    output.push_str("Err(");
                    value.write(debug, output);
                    output.push(')');
                }
            },
            Self::Function(function) => {
                output.push_str("<fn ");
                output.push_str(function.name());
                output.push('>');
            }
            Self::Builtin(function) => {
                output.push_str("<builtin ");
                output.push_str(builtin_name(*function));
                output.push('>');
            }
            Self::BoundMethod { receiver, name } => {
                receiver.write(debug, output);
                output.push('.');
                output.push_str(name);
                output.push_str("()");
            }
            Self::Constructor(constructor) => {
                output.push_str("<constructor ");
                output.push_str(constructor_name(&constructor.kind));
                output.push('>');
            }
            Self::Task(_) => output.push_str("<Task>"),
            Self::Channel(_) => output.push_str("<Channel>"),
            Self::ForeignModule(name) => {
                output.push_str("<foreign module ");
                output.push_str(name);
                output.push('>');
            }
            Self::ScriptBlock { name, language, .. } => {
                output.push_str("<script ");
                output.push_str(name);
                output.push(':');
                output.push_str(language);
                output.push('>');
            }
            Self::ScriptFunction {
                script_name,
                function_name,
                ..
            } => {
                output.push_str("<script function ");
                output.push_str(script_name);
                output.push('.');
                output.push_str(function_name);
                output.push_str("()>");
            }
            Self::ModuleNamespace(name) => {
                output.push_str("<module ");
                output.push_str(name);
                output.push('>');
            }
            Self::EnumNamespace { module, enum_name } => {
                output.push_str("<enum ");
                output.push_str(module);
                output.push_str("::");
                output.push_str(enum_name);
                output.push('>');
            }
            Self::Imported { module, name } => {
                output.push_str("<imported ");
                output.push_str(module);
                output.push_str("::");
                output.push_str(name);
                output.push('>');
            }
            Self::Failure(value) => {
                output.push_str("<failure ");
                value.write(debug, output);
                output.push('>');
            }
        }
    }
}

fn write_float(value: f64, output: &mut String) {
    if value.is_nan() {
        output.push_str("NaN");
    } else if value == f64::INFINITY {
        output.push_str("Infinity");
    } else if value == f64::NEG_INFINITY {
        output.push_str("-Infinity");
    } else if value.fract() == 0.0 && value.abs() < 1e16 {
        write!(output, "{value:.1}").expect("writing to a String cannot fail");
    } else {
        output.push_str(&value.to_string());
    }
}

fn write_bytes(bytes: &[u8], output: &mut String) {
    output.push_str("b\"");
    for byte in bytes {
        match byte {
            b'\\' => output.push_str("\\\\"),
            b'"' => output.push_str("\\\""),
            b'\n' => output.push_str("\\n"),
            b'\r' => output.push_str("\\r"),
            b'\t' => output.push_str("\\t"),
            0x20..=0x7e => output.push(char::from(*byte)),
            _ => write!(output, "\\u{{{byte:02x}}}").expect("writing to a String cannot fail"),
        }
    }
    output.push('"');
}

fn builtin_name(function: BuiltinFunction) -> &'static str {
    match function {
        BuiltinFunction::Print => "print",
        BuiltinFunction::Println => "println",
        BuiltinFunction::Dbg => "dbg",
        BuiltinFunction::Assert => "assert",
    }
}

fn constructor_name(constructor: &ConstructorKind) -> &str {
    match constructor {
        ConstructorKind::Some => "Some",
        ConstructorKind::Ok => "Ok",
        ConstructorKind::Err => "Err",
        ConstructorKind::BytesNew => "bytes.new",
        ConstructorKind::ChannelCreate => "Channel.create",
        ConstructorKind::TupleStruct { name, .. } => name,
        ConstructorKind::EnumVariant { variant, .. } => variant,
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.render_debug())
    }
}

impl fmt::Display for Value {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.render())
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        values_equal(self, other)
    }
}

#[must_use]
pub fn values_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Unit, Value::Unit) => true,
        (Value::Bool(left), Value::Bool(right)) => left == right,
        (Value::Integer(left), Value::Integer(right)) => {
            left.ty == right.ty && left.value == right.value
        }
        (Value::Float(left), Value::Float(right)) => {
            left.ty == right.ty && left.value == right.value
        }
        (Value::Str(left), Value::Str(right)) => left == right,
        (Value::Bytes(left), Value::Bytes(right)) => *left.borrow() == *right.borrow(),
        (Value::Array(left), Value::Array(right)) => {
            Rc::ptr_eq(left, right) || *left.borrow() == *right.borrow()
        }
        (Value::Map(left), Value::Map(right)) => {
            Rc::ptr_eq(left, right) || maps_equal(&left.borrow(), &right.borrow())
        }
        (Value::Tuple(left), Value::Tuple(right)) => {
            Rc::ptr_eq(left, right) || *left.borrow() == *right.borrow()
        }
        (Value::Struct(left), Value::Struct(right)) => {
            Rc::ptr_eq(left, right) || structs_equal(&left.borrow(), &right.borrow())
        }
        (Value::Enum(left), Value::Enum(right)) => {
            Rc::ptr_eq(left, right) || *left.borrow() == *right.borrow()
        }
        (Value::Option(left), Value::Option(right)) => left == right,
        (Value::Result(left), Value::Result(right)) => left == right,
        (Value::Function(left), Value::Function(right)) => Rc::ptr_eq(left, right),
        (Value::Builtin(left), Value::Builtin(right)) => left == right,
        (
            Value::BoundMethod {
                receiver: left_receiver,
                name: left_name,
            },
            Value::BoundMethod {
                receiver: right_receiver,
                name: right_name,
            },
        ) => left_name == right_name && values_equal(left_receiver, right_receiver),
        (Value::Constructor(left), Value::Constructor(right)) => left == right,
        (Value::Task(left), Value::Task(right)) => Rc::ptr_eq(left, right),
        (Value::Channel(left), Value::Channel(right)) => Rc::ptr_eq(left, right),
        (Value::ForeignModule(left), Value::ForeignModule(right)) => left == right,
        (
            Value::ScriptBlock {
                name: l_name,
                language: l_lang,
                ..
            },
            Value::ScriptBlock {
                name: r_name,
                language: r_lang,
                ..
            },
        ) => l_name == r_name && l_lang == r_lang,
        (
            Value::ScriptFunction {
                script_name: l_s,
                function_name: l_f,
                ..
            },
            Value::ScriptFunction {
                script_name: r_s,
                function_name: r_f,
                ..
            },
        ) => l_s == r_s && l_f == r_f,
        (Value::ModuleNamespace(left), Value::ModuleNamespace(right)) => left == right,
        (
            Value::EnumNamespace {
                module: left_module,
                enum_name: left_enum,
            },
            Value::EnumNamespace {
                module: right_module,
                enum_name: right_enum,
            },
        ) => left_module == right_module && left_enum == right_enum,
        (
            Value::Imported {
                module: left_module,
                name: left_name,
            },
            Value::Imported {
                module: right_module,
                name: right_name,
            },
        ) => left_module == right_module && left_name == right_name,
        (Value::Failure(left), Value::Failure(right)) => values_equal(left, right),
        _ => false,
    }
}

fn maps_equal(left: &[(Value, Value)], right: &[(Value, Value)]) -> bool {
    left.len() == right.len()
        && left.iter().all(|(left_key, left_value)| {
            right
                .iter()
                .find(|(right_key, _)| values_equal(left_key, right_key))
                .is_some_and(|(_, right_value)| values_equal(left_value, right_value))
        })
}

fn structs_equal(left: &StructValue, right: &StructValue) -> bool {
    if left.name != right.name {
        return false;
    }
    match (&left.fields, &right.fields) {
        (StructFieldsValue::Tuple(left), StructFieldsValue::Tuple(right)) => left == right,
        (StructFieldsValue::Named(left), StructFieldsValue::Named(right)) => {
            left.len() == right.len()
                && left.iter().all(|(name, value)| {
                    right
                        .iter()
                        .find(|(right_name, _)| name == right_name)
                        .is_some_and(|(_, right_value)| values_equal(value, right_value))
                })
        }
        _ => false,
    }
}
