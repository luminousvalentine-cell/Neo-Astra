use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Type {
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    Usize,
    Isize,
    F32,
    F64,
    Bool,
    Str,
    Bytes,
    Unit,
    Never,
    Option(Box<Type>),
    Result(Box<Type>, Box<Type>),
    Array(Box<Type>),
    FixedArray(Box<Type>, usize),
    Slice(Box<Type>),
    Map(Box<Type>, Box<Type>),
    Tuple(Vec<Type>),
    Function(FunctionType),

    Named(String, Vec<Type>),

    Generic(String),

    Error,

    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct FunctionType {
    pub type_parameters: Vec<String>,
    pub parameters: Vec<Type>,
    pub return_type: Box<Type>,
    pub errors: Vec<Type>,
}

impl Type {

    #[must_use]
    pub const fn unit() -> Self {
        Self::Unit
    }

    #[must_use]
    pub const fn is_integer(&self) -> bool {
        matches!(
            self,
            Self::I8
                | Self::I16
                | Self::I32
                | Self::I64
                | Self::U8
                | Self::U16
                | Self::U32
                | Self::U64
                | Self::Usize
                | Self::Isize
        )
    }

    #[must_use]
    pub const fn is_float(&self) -> bool {
        matches!(self, Self::F32 | Self::F64)
    }

    #[must_use]
    pub const fn is_numeric(&self) -> bool {
        self.is_integer() || self.is_float()
    }

    #[must_use]
    pub const fn is_opaque(&self) -> bool {
        matches!(self, Self::Unknown | Self::Error)
    }

    #[must_use]
    pub fn substitute(&self, substitutions: &std::collections::BTreeMap<String, Type>) -> Self {
        match self {
            Self::Generic(name) => substitutions
                .get(name)
                .cloned()
                .unwrap_or_else(|| self.clone()),
            Self::Option(inner) => Self::Option(Box::new(inner.substitute(substitutions))),
            Self::Result(ok, error) => Self::Result(
                Box::new(ok.substitute(substitutions)),
                Box::new(error.substitute(substitutions)),
            ),
            Self::Array(inner) => Self::Array(Box::new(inner.substitute(substitutions))),
            Self::FixedArray(inner, length) => {
                Self::FixedArray(Box::new(inner.substitute(substitutions)), *length)
            }
            Self::Slice(inner) => Self::Slice(Box::new(inner.substitute(substitutions))),
            Self::Map(key, value) => Self::Map(
                Box::new(key.substitute(substitutions)),
                Box::new(value.substitute(substitutions)),
            ),
            Self::Tuple(elements) => Self::Tuple(
                elements
                    .iter()
                    .map(|element| element.substitute(substitutions))
                    .collect(),
            ),
            Self::Function(function) => Self::Function(FunctionType {
                type_parameters: function.type_parameters.clone(),
                parameters: function
                    .parameters
                    .iter()
                    .map(|parameter| parameter.substitute(substitutions))
                    .collect(),
                return_type: Box::new(function.return_type.substitute(substitutions)),
                errors: function
                    .errors
                    .iter()
                    .map(|error| error.substitute(substitutions))
                    .collect(),
            }),
            Self::Named(name, arguments) => Self::Named(
                name.clone(),
                arguments
                    .iter()
                    .map(|argument| argument.substitute(substitutions))
                    .collect(),
            ),
            _ => self.clone(),
        }
    }

    #[must_use]
    pub fn accepts(&self, actual: &Self) -> bool {
        if self.is_opaque() || actual.is_opaque() || actual == &Self::Never {
            return true;
        }

        match (self, actual) {
            (Self::Option(expected), Self::Option(actual)) => expected.accepts(actual),
            (Self::Result(expected_ok, expected_error), Self::Result(actual_ok, actual_error)) => {
                expected_ok.accepts(actual_ok) && expected_error.accepts(actual_error)
            }
            (Self::Array(expected), Self::Array(actual))
            | (Self::Slice(expected), Self::Array(actual))
            | (Self::Slice(expected), Self::Slice(actual))
            | (Self::Array(expected), Self::Slice(actual)) => expected.accepts(actual),
            (
                Self::FixedArray(expected, expected_length),
                Self::FixedArray(actual, actual_length),
            ) => expected_length == actual_length && expected.accepts(actual),
            (Self::Map(expected_key, expected_value), Self::Map(actual_key, actual_value)) => {
                expected_key.accepts(actual_key) && expected_value.accepts(actual_value)
            }
            (Self::Tuple(expected), Self::Tuple(actual)) => {
                expected.len() == actual.len()
                    && expected
                        .iter()
                        .zip(actual)
                        .all(|(expected, actual)| expected.accepts(actual))
            }
            (Self::Function(expected), Self::Function(actual)) => {
                expected.type_parameters == actual.type_parameters
                    && expected.parameters.len() == actual.parameters.len()
                    && expected.parameters.iter().zip(&actual.parameters).all(
                        |(expected, actual)| expected.accepts(actual) && actual.accepts(expected),
                    )
                    && expected.return_type.accepts(&actual.return_type)
                    && expected.errors.len() == actual.errors.len()
                    && expected
                        .errors
                        .iter()
                        .zip(&actual.errors)
                        .all(|(expected, actual)| expected.accepts(actual))
            }
            (Self::Named(expected_name, expected_args), Self::Named(actual_name, actual_args)) => {
                expected_name == actual_name
                    && expected_args.len() == actual_args.len()
                    && expected_args
                        .iter()
                        .zip(actual_args)
                        .all(|(expected, actual)| expected.accepts(actual))
            }
            _ => self == actual,
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::I8 => formatter.write_str("i8"),
            Self::I16 => formatter.write_str("i16"),
            Self::I32 => formatter.write_str("i32"),
            Self::I64 => formatter.write_str("i64"),
            Self::U8 => formatter.write_str("u8"),
            Self::U16 => formatter.write_str("u16"),
            Self::U32 => formatter.write_str("u32"),
            Self::U64 => formatter.write_str("u64"),
            Self::Usize => formatter.write_str("usize"),
            Self::Isize => formatter.write_str("isize"),
            Self::F32 => formatter.write_str("f32"),
            Self::F64 => formatter.write_str("f64"),
            Self::Bool => formatter.write_str("bool"),
            Self::Str => formatter.write_str("str"),
            Self::Bytes => formatter.write_str("bytes"),
            Self::Unit => formatter.write_str("()"),
            Self::Never => formatter.write_str("Never"),
            Self::Option(inner) => write!(formatter, "Option<{inner}>"),
            Self::Result(ok, error) => write!(formatter, "Result<{ok}, {error}>"),
            Self::Array(inner) => write!(formatter, "[{inner}]"),
            Self::FixedArray(inner, length) => write!(formatter, "[{inner}; {length}]"),
            Self::Slice(inner) => write!(formatter, "[{inner}]"),
            Self::Map(key, value) => write!(formatter, "{{{key}: {value}}}"),
            Self::Tuple(elements) => {
                formatter.write_str("(")?;
                for (index, element) in elements.iter().enumerate() {
                    if index > 0 {
                        formatter.write_str(", ")?;
                    }
                    write!(formatter, "{element}")?;
                }
                formatter.write_str(")")
            }
            Self::Function(function) => {
                write!(formatter, "fn(")?;
                for (index, parameter) in function.parameters.iter().enumerate() {
                    if index > 0 {
                        formatter.write_str(", ")?;
                    }
                    write!(formatter, "{parameter}")?;
                }
                write!(formatter, ") -> {}", function.return_type)?;
                if !function.errors.is_empty() {
                    formatter.write_str(" ! ")?;
                    for (index, error) in function.errors.iter().enumerate() {
                        if index > 0 {
                            formatter.write_str(" | ")?;
                        }
                        write!(formatter, "{error}")?;
                    }
                }
                Ok(())
            }
            Self::Named(name, arguments) => {
                formatter.write_str(name)?;
                if !arguments.is_empty() {
                    formatter.write_str("<")?;
                    for (index, argument) in arguments.iter().enumerate() {
                        if index > 0 {
                            formatter.write_str(", ")?;
                        }
                        write!(formatter, "{argument}")?;
                    }
                    formatter.write_str(">")?;
                }
                Ok(())
            }
            Self::Generic(name) => formatter.write_str(name),
            Self::Error => formatter.write_str("<error>"),
            Self::Unknown => formatter.write_str("<unknown>"),
        }
    }
}
