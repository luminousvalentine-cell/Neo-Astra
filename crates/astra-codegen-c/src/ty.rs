use std::collections::{BTreeMap, BTreeSet};

use astra_core::ast::{self, EnumDecl, StructDecl, StructFields, TypeKind};
use astra_core::span::Span;
use astra_core::types::Type;

const BUILTIN_NOMINALS: &[&str] = &[
    "SliceError",
    "ParseIntError",
    "OverflowError",
    "ChannelError",
    "TaskError",
];

fn builtin_nominal(name: &str) -> Option<&'static str> {
    BUILTIN_NOMINALS
        .iter()
        .copied()
        .find(|builtin| *builtin == name)
}

#[derive(Debug, Clone)]
pub enum DeclShape {
    Struct(StructShape),
    Enum(EnumShape),
}

#[derive(Debug, Clone)]
pub struct StructShape {
    pub tuple: bool,
    pub fields: Vec<(String, Type)>,
}

#[derive(Debug, Clone)]
pub struct EnumShape {
    pub variants: Vec<EnumVariantShape>,
}

#[derive(Debug, Clone)]
pub struct EnumVariantShape {
    pub name: String,
    pub fields: Vec<Type>,
}

enum OwnedDecl {
    Struct(Vec<(String, Type)>),
    Enum(Vec<EnumVariantShape>),
}

#[derive(Debug, Clone, Default)]
pub struct TypeEnv {
    pub decls: BTreeMap<String, DeclShape>,
}

impl TypeEnv {
    pub fn collect(module: &ast::Module) -> Self {
        let mut decls = BTreeMap::new();
        for item in &module.items {
            match &item.kind {
                ast::ItemKind::Struct(s) => {
                    decls.insert(s.name.clone(), DeclShape::Struct(struct_shape(s)));
                }
                ast::ItemKind::Enum(e) => {
                    decls.insert(e.name.clone(), DeclShape::Enum(enum_shape(e)));
                }
                _ => {}
            }
        }
        Self { decls }
    }

    pub fn shape(&self, name: &str) -> Option<&DeclShape> {
        self.decls.get(name)
    }
}

#[derive(Debug, Clone)]
pub struct ForeignDecl {
    pub stem: String,

    pub local: String,
    pub shape: DeclShape,
}

#[derive(Debug, Clone)]
pub struct Nominal {
    pub prefix: String,

    pub source: String,
    pub shape: DeclShape,

    pub foreign: bool,
}

fn struct_shape(decl: &StructDecl) -> StructShape {
    match &decl.fields {
        StructFields::Named(fields) => StructShape {
            tuple: false,
            fields: fields
                .iter()
                .map(|field| (field.name.clone(), to_type(&field.ty)))
                .collect(),
        },
        StructFields::Tuple(types) => StructShape {
            tuple: true,
            fields: types
                .iter()
                .enumerate()
                .map(|(index, ty)| (format!("_{index}"), to_type(ty)))
                .collect(),
        },
    }
}

fn enum_shape(decl: &EnumDecl) -> EnumShape {
    EnumShape {
        variants: decl
            .variants
            .iter()
            .map(|variant| EnumVariantShape {
                name: variant.name.clone(),
                fields: variant
                    .fields
                    .iter()
                    .flatten()
                    .map(|field| to_type(&field.ty))
                    .collect(),
            })
            .collect(),
    }
}

pub fn to_type(ast_type: &ast::Type) -> Type {
    match &ast_type.kind {
        TypeKind::Path(path) => path_to_type(path),
        TypeKind::Array(inner) => Type::Array(Box::new(to_type(inner))),
        TypeKind::FixedArray(inner, length) => {
            Type::FixedArray(Box::new(to_type(inner)), length.parse().unwrap_or(0))
        }
        TypeKind::Map(key, value) => Type::Map(Box::new(to_type(key)), Box::new(to_type(value))),
        TypeKind::Tuple(elements) => Type::Tuple(elements.iter().map(to_type).collect()),
        TypeKind::Function {
            parameters,
            return_type,
            error_set,
        } => Type::Function(astra_core::types::FunctionType {
            type_parameters: Vec::new(),
            parameters: parameters.iter().map(to_type).collect(),
            return_type: Box::new(
                return_type
                    .as_ref()
                    .map(|t| to_type(t))
                    .unwrap_or(Type::Unit),
            ),
            errors: error_set.iter().map(path_to_type).collect(),
        }),
        TypeKind::Unit => Type::Unit,
    }
}

pub fn path_to_type(path: &ast::TypePath) -> Type {
    let name = path.segments.last().map(String::as_str).unwrap_or("");
    if path.arguments.is_empty()
        && let Some(primitive) = primitive_type(name)
    {
        return primitive;
    }
    match name {
        "Option" if path.arguments.len() == 1 => {
            Type::Option(Box::new(to_type(&path.arguments[0])))
        }
        "Result" if path.arguments.len() == 2 => Type::Result(
            Box::new(to_type(&path.arguments[0])),
            Box::new(to_type(&path.arguments[1])),
        ),
        _ => Type::Named(
            path.segments.join("."),
            path.arguments.iter().map(to_type).collect(),
        ),
    }
}

fn primitive_type(name: &str) -> Option<Type> {
    Some(match name {
        "i8" => Type::I8,
        "i16" => Type::I16,
        "i32" => Type::I32,
        "i64" => Type::I64,
        "u8" => Type::U8,
        "u16" => Type::U16,
        "u32" => Type::U32,
        "u64" => Type::U64,
        "usize" => Type::Usize,
        "isize" => Type::Isize,
        "f32" => Type::F32,
        "f64" => Type::F64,
        "bool" => Type::Bool,
        "str" => Type::Str,
        "bytes" => Type::Bytes,
        _ => return None,
    })
}

pub fn sanitize(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut last_under = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_under = false;
        } else if !last_under {
            out.push('_');
            last_under = true;
        }
    }
    if out.is_empty() {
        out.push('_');
    }
    out
}

#[derive(Debug, Clone)]
pub struct Reject {
    pub code: &'static str,
    pub message: String,
}

pub struct TyCtxt {
    pub env: TypeEnv,
    seen: BTreeSet<String>,
    pub typedefs: Vec<String>,

    pub shapes: Vec<String>,
    shape_keys: BTreeMap<String, String>,
    pub rejects: Vec<Reject>,

    type_prefix: String,

    foreign: BTreeMap<String, ForeignDecl>,

    subst: BTreeMap<String, Type>,
}

impl TyCtxt {
    pub fn new(env: TypeEnv) -> Self {
        Self {
            env,
            seen: BTreeSet::new(),
            typedefs: Vec::new(),
            shapes: Vec::new(),
            shape_keys: BTreeMap::new(),
            rejects: Vec::new(),
            type_prefix: String::new(),
            foreign: BTreeMap::new(),
            subst: BTreeMap::new(),
        }
    }

    pub fn set_subst(&mut self, subst: BTreeMap<String, Type>) {
        self.subst = subst;
    }

    pub fn clear_subst(&mut self) {
        self.subst.clear();
    }

    pub fn substitute(&self, ty: &Type) -> Type {
        if self.subst.is_empty() {
            return ty.clone();
        }
        match ty {
            Type::Generic(name) => match self.subst.get(name) {
                Some(concrete) => concrete.clone(),
                None => ty.clone(),
            },
            Type::Named(name, arguments) if arguments.is_empty() => match self.subst.get(name) {
                Some(concrete) => concrete.clone(),
                None => ty.clone(),
            },
            Type::Option(inner) => Type::Option(Box::new(self.substitute(inner))),
            Type::Array(inner) => Type::Array(Box::new(self.substitute(inner))),
            Type::Slice(inner) => Type::Slice(Box::new(self.substitute(inner))),
            Type::FixedArray(inner, length) => {
                Type::FixedArray(Box::new(self.substitute(inner)), *length)
            }
            Type::Result(ok, error) => Type::Result(
                Box::new(self.substitute(ok)),
                Box::new(self.substitute(error)),
            ),
            Type::Map(key, value) => Type::Map(
                Box::new(self.substitute(key)),
                Box::new(self.substitute(value)),
            ),
            Type::Tuple(elements) => Type::Tuple(
                elements
                    .iter()
                    .map(|element| self.substitute(element))
                    .collect(),
            ),
            Type::Named(name, arguments) => Type::Named(
                name.clone(),
                arguments
                    .iter()
                    .map(|argument| self.substitute(argument))
                    .collect(),
            ),
            _ => ty.clone(),
        }
    }

    pub fn add_foreign(&mut self, name: String, decl: ForeignDecl) {
        self.foreign.insert(name, decl);
    }

    pub fn nominal(&self, name: &str) -> Option<Nominal> {
        if let Some(shape) = self.env.shape(name).cloned() {
            return Some(Nominal {
                prefix: self.type_prefix.clone(),
                source: name.to_string(),
                shape,
                foreign: false,
            });
        }
        let decl = self.foreign.get(name)?;
        Some(Nominal {
            prefix: if decl.stem.is_empty() {
                String::new()
            } else {
                format!("{}_", decl.stem)
            },
            source: decl.local.clone(),
            shape: decl.shape.clone(),
            foreign: true,
        })
    }

    pub fn struct_shape(&self, name: &str) -> Option<StructShape> {
        match self.nominal(name)?.shape {
            DeclShape::Struct(shape) => Some(shape),
            DeclShape::Enum(_) => None,
        }
    }

    pub fn enum_shape(&self, name: &str) -> Option<EnumShape> {
        match self.nominal(name)?.shape {
            DeclShape::Enum(shape) => Some(shape),
            DeclShape::Struct(_) => None,
        }
    }

    pub fn enum_tag(&self, owner: &str, variant: &str) -> Option<String> {
        let nominal = self.nominal(owner)?;
        if !matches!(nominal.shape, DeclShape::Enum(_)) {
            return None;
        }
        Some(format!(
            "NS_{}{}_TAG_{}",
            nominal.prefix,
            sanitize(&nominal.source),
            sanitize(variant)
        ))
    }

    pub fn set_type_prefix(&mut self, stem: &str) {
        self.type_prefix = if stem.is_empty() {
            String::new()
        } else {
            format!("{stem}_")
        };
    }

    pub fn rejected(&self) -> bool {
        !self.rejects.is_empty()
    }

    pub fn ensure(&mut self, name: &str, decl: String) {
        if self.seen.insert(name.to_string()) {
            self.typedefs.push(decl);
        }
    }

    pub fn ident(&self, ty: &Type) -> String {
        if let Type::Generic(name) = ty
            && let Some(concrete) = self.subst.get(name)
        {
            return self.ident(concrete);
        }
        if let Type::Named(name, arguments) = ty
            && arguments.is_empty()
            && let Some(concrete) = self.subst.get(name)
        {
            return self.ident(concrete);
        }
        match ty {
            Type::I8 => "i8".into(),
            Type::I16 => "i16".into(),
            Type::I32 => "i32".into(),
            Type::I64 => "i64".into(),
            Type::U8 => "u8".into(),
            Type::U16 => "u16".into(),
            Type::U32 => "u32".into(),
            Type::U64 => "u64".into(),
            Type::Usize => "usize".into(),
            Type::Isize => "isize".into(),
            Type::F32 => "f32".into(),
            Type::F64 => "f64".into(),
            Type::Bool => "bool".into(),
            Type::Str => "Str".into(),
            Type::Bytes => "Bytes".into(),
            Type::Unit => "Unit".into(),
            Type::Never => "Never".into(),
            Type::Error => "Error".into(),
            Type::Unknown => "Unknown".into(),
            Type::Option(inner) => format!("Option_{}", self.ident(inner)),
            Type::Result(ok, err) => format!("Result_{}_{}", self.ident(ok), self.ident(err)),
            Type::Array(inner) => format!("Array_{}", self.ident(inner)),
            Type::Slice(inner) => format!("Slice_{}", self.ident(inner)),
            Type::FixedArray(inner, n) => format!("FixedArray_{}_{}", self.ident(inner), n),
            Type::Map(key, value) => format!("Map_{}_{}", self.ident(key), self.ident(value)),
            Type::Tuple(elements) => {
                format!(
                    "Tuple{}_{}",
                    elements.len(),
                    elements
                        .iter()
                        .map(|t| self.ident(t))
                        .collect::<Vec<_>>()
                        .join("_")
                )
            }
            Type::Function(_) => "Fn".into(),
            Type::Named(name, args) => {
                let base = match self.nominal(name) {
                    Some(nominal) => format!("{}{}", nominal.prefix, sanitize(&nominal.source)),

                    None => match builtin_nominal(name) {
                        Some(builtin) => builtin.to_string(),
                        None => format!("{}{}", self.type_prefix, sanitize(name)),
                    },
                };
                if args.is_empty() {
                    base
                } else {
                    format!(
                        "{}_{}",
                        base,
                        args.iter()
                            .map(|t| self.ident(t))
                            .collect::<Vec<_>>()
                            .join("_")
                    )
                }
            }
            Type::Generic(name) => sanitize(name),
        }
    }

    pub fn c_type(&mut self, ty: &Type) -> String {
        if let Type::Generic(name) = ty
            && let Some(concrete) = self.subst.get(name).cloned()
        {
            return self.c_type(&concrete);
        }
        if let Type::Named(name, arguments) = ty
            && arguments.is_empty()
            && let Some(concrete) = self.subst.get(name).cloned()
            && concrete != *ty
        {
            return self.c_type(&concrete);
        }
        match ty {
            Type::I8 => "int8_t".into(),
            Type::I16 => "int16_t".into(),
            Type::I32 => "int32_t".into(),
            Type::I64 => "int64_t".into(),
            Type::U8 => "uint8_t".into(),
            Type::U16 => "uint16_t".into(),
            Type::U32 => "uint32_t".into(),
            Type::U64 => "uint64_t".into(),
            Type::Usize => "size_t".into(),
            Type::Isize => "intptr_t".into(),
            Type::F32 => "float".into(),
            Type::F64 => "double".into(),
            Type::Bool => "bool".into(),
            Type::Str => "NS_Str".into(),
            Type::Bytes => "NS_Bytes".into(),
            Type::Unit => "NS_Unit".into(),
            Type::Never => {
                self.rejects.push(Reject {
                    code: "NS8099",
                    message: "Never appears as a stored type, which the C backend cannot emit"
                        .into(),
                });
                "void".into()
            }
            Type::Option(inner) => {
                let value = self.c_type(inner);
                let name = format!("NS_Option_{}", self.ident(inner));
                self.ensure(
                    &name,
                    format!("typedef struct {{ bool some; {value} value; }} {name};"),
                );
                name
            }
            Type::Result(ok, err) => {
                let value = self.c_type(ok);
                let error = self.c_type(err);
                let name = format!("NS_Result_{}_{}", self.ident(ok), self.ident(err));
                self.ensure(
                    &name,
                    format!("typedef struct {{ bool ok; {value} value; {error} error; }} {name};"),
                );
                name
            }
            Type::Array(_) | Type::Slice(_) => "NS_Array".into(),
            Type::FixedArray(_, _) => {
                self.rejects.push(Reject {
                    code: "NS8022",
                    message: "fixed-size arrays are not supported by the C backend first slice"
                        .into(),
                });
                "NS_Array".into()
            }
            Type::Map(_, _) => "NS_Map".into(),
            Type::Tuple(elements) => {
                let name = format!("NS_{}", self.ident(ty));
                let mut fields = String::new();
                for (index, element) in elements.iter().enumerate() {
                    let field = self.c_type(element);
                    fields.push_str(&format!("{field} _{index}; "));
                }
                self.ensure(&name, format!("typedef struct {{ {fields}}} {name};"));
                name
            }
            Type::Function(_) => {
                self.rejects.push(Reject {
                    code: "NS8017",
                    message: "function values are not supported by the C backend first slice"
                        .into(),
                });
                "void*".into()
            }
            Type::Named(name, _) if name == "Channel" => "NS_Channel*".into(),

            Type::Named(name, _) if name == "Region" => "NS_Region*".into(),
            Type::Named(name, args) if name == "Ptr" => {
                let inner = args.first().cloned().unwrap_or(Type::U8);
                let value = self.c_type(&inner);
                format!("{value}*")
            }
            Type::Named(name, args) if name == "Task" => {
                let inner = args.first().cloned().unwrap_or(Type::Unit);
                let struct_name = self.task_struct_ref(&inner);
                format!("{struct_name}*")
            }
            Type::Named(name, args) => {
                if !args.is_empty() {
                    self.rejects.push(Reject {
                        code: "NS8018",
                        message: format!(
                            "generic type '{name}' requires monomorphization, unsupported by the C backend first slice"
                        ),
                    });
                    return format!("NS_{}", sanitize(name));
                }
                self.user_typedef(name)
            }
            Type::Generic(name) => {
                self.rejects.push(Reject {
                    code: "NS8018",
                    message: format!(
                        "generic parameter '{name}' requires monomorphization, unsupported by the C backend first slice"
                    ),
                });
                sanitize(name)
            }
            Type::Error | Type::Unknown => {
                self.rejects.push(Reject {
                    code: "NS8099",
                    message: "an unresolved or recovery type reached the C backend".into(),
                });
                "void*".into()
            }
        }
    }

    pub fn block_shape_ref(&mut self, elem: &Type) -> String {
        let elem_shape = self.shape_ref(elem);
        if elem_shape == "&NS_SHAPE_LEAF" {
            return "&NS_SHAPE_LEAF".to_owned();
        }
        let elem_c = self.c_type(elem);
        let template = format!(
            "static const NS_Gc_Shape {{NAME}} = {{ .kind = NS_SHAPE_KIND_ARRAY, \
             .elem_size = sizeof({elem_c}), .elem = {elem_shape} }};"
        );
        self.define_shape(&template)
    }

    pub fn channel_handle_shape_ref(&mut self, elem: &Type) -> String {
        let data = self.block_shape_ref(elem);
        self.record_shape(&format!("{{ offsetof(NS_Channel, data), {data} }}"), 1)
    }

    pub fn value_shape_ref(&mut self, ty: &Type) -> String {
        self.shape_ref(ty)
    }

    pub fn task_struct_ref(&mut self, inner: &Type) -> String {
        let value = self.c_type(inner);
        let name = format!("NS_Task_{}", self.ident(inner));
        self.ensure(
            &name,
            format!("typedef struct {{ NS_Task_Core core; {value} value; }} {name};"),
        );
        name
    }

    pub fn task_block_shape_ref(&mut self, inner: &Type) -> String {
        let value_shape = self.shape_ref(inner);
        if value_shape == "&NS_SHAPE_LEAF" {
            return "&NS_SHAPE_LEAF".to_owned();
        }
        let task = self.task_struct_ref(inner);
        self.record_shape(&format!("{{ offsetof({task}, value), {value_shape} }}"), 1)
    }

    fn shape_ref(&mut self, ty: &Type) -> String {
        if let Type::Generic(name) = ty
            && let Some(concrete) = self.subst.get(name).cloned()
        {
            return self.shape_ref(&concrete);
        }
        if let Type::Named(name, arguments) = ty
            && arguments.is_empty()
            && let Some(concrete) = self.subst.get(name).cloned()
            && concrete != *ty
        {
            return self.shape_ref(&concrete);
        }
        match ty {
            Type::Str | Type::Bytes => "&NS_SHAPE_STR".to_owned(),
            Type::Array(inner) | Type::Slice(inner) => {
                let data = self.block_shape_ref(inner);
                self.record_shape(&format!("{{ offsetof(NS_Array, data), {data} }}"), 1)
            }
            Type::Map(key, value) => {
                let key_shape = self.block_shape_ref(key);
                let value_shape = self.block_shape_ref(value);
                self.record_shape(
                    &format!(
                        "{{ offsetof(NS_Map, keys), {key_shape} }}, \
                         {{ offsetof(NS_Map, values), {value_shape} }}"
                    ),
                    2,
                )
            }
            Type::Option(inner) => {
                let c = self.c_type(ty);
                let inner_shape = self.shape_ref(inner);
                let payload = if inner_shape == "&NS_SHAPE_LEAF" {
                    "NULL".to_owned()
                } else {
                    self.record_shape(&format!("{{ offsetof({c}, value), {inner_shape} }}"), 1)
                };
                self.variant_shape(&c, &[payload, "NULL".to_owned()])
            }
            Type::Result(ok, error) => {
                let c = self.c_type(ty);
                let error_shape = self.shape_ref(error);
                let error_payload = if error_shape == "&NS_SHAPE_LEAF" {
                    "NULL".to_owned()
                } else {
                    self.record_shape(&format!("{{ offsetof({c}, error), {error_shape} }}"), 1)
                };
                let ok_shape = self.shape_ref(ok);
                let ok_payload = if ok_shape == "&NS_SHAPE_LEAF" {
                    "NULL".to_owned()
                } else {
                    self.record_shape(&format!("{{ offsetof({c}, value), {ok_shape} }}"), 1)
                };
                self.variant_shape(&c, &[error_payload, ok_payload])
            }
            Type::Named(name, args) => match name.as_str() {
                "Ptr" | "Region" => "&NS_SHAPE_LEAF".to_owned(),

                "Channel" => "&NS_SHAPE_LEAF".to_owned(),
                "Task" => self.record_shape("{ 0, &NS_SHAPE_LEAF }", 1),

                "SliceError" | "ParseIntError" | "OverflowError" | "TaskError" => {
                    let c = self.c_type(ty);
                    self.record_shape(&format!("{{ offsetof({c}, _0), &NS_SHAPE_STR }}"), 1)
                }
                _ => {
                    if !args.is_empty() {
                        return "&NS_SHAPE_LEAF".to_owned();
                    }
                    match self.env.shape(name).cloned() {
                        Some(DeclShape::Struct(struct_shape)) => {
                            let c = self.c_type(ty);
                            let mut fields: Vec<String> = Vec::new();
                            for (field_name, field_type) in &struct_shape.fields {
                                let field_shape = self.shape_ref(field_type);
                                if field_shape != "&NS_SHAPE_LEAF" {
                                    fields.push(format!(
                                        "{{ offsetof({c}, {field_name}), {field_shape} }}"
                                    ));
                                }
                            }
                            if fields.is_empty() {
                                "&NS_SHAPE_LEAF".to_owned()
                            } else {
                                self.record_shape(&fields.join(", "), fields.len())
                            }
                        }
                        Some(DeclShape::Enum(enum_shape)) => {
                            let c = self.c_type(ty);
                            let mut variants: Vec<String> = Vec::new();
                            for variant in &enum_shape.variants {
                                let mut fields: Vec<String> = Vec::new();
                                for (index, field_type) in variant.fields.iter().enumerate() {
                                    let field_shape = self.shape_ref(field_type);
                                    if field_shape != "&NS_SHAPE_LEAF" {
                                        let offset = if variant.fields.len() == 1 {
                                            format!("offsetof({c}, payload.{})", variant.name)
                                        } else {
                                            format!(
                                                "offsetof({c}, payload.{}._{index})",
                                                variant.name
                                            )
                                        };
                                        fields.push(format!("{{ {offset}, {field_shape} }}"));
                                    }
                                }
                                if fields.is_empty() {
                                    variants.push("NULL".to_owned());
                                } else {
                                    variants
                                        .push(self.record_shape(&fields.join(", "), fields.len()));
                                }
                            }
                            self.variant_shape(&c, &variants)
                        }
                        _ => "&NS_SHAPE_LEAF".to_owned(),
                    }
                }
            },

            _ => "&NS_SHAPE_LEAF".to_owned(),
        }
    }

    pub fn record_shape_ref(&mut self, fields: &str, count: usize) -> String {
        self.record_shape(fields, count)
    }

    fn record_shape(&mut self, fields: &str, count: usize) -> String {
        let template = format!(
            "static const NS_Gc_Shape_Field {{NAME}}_f[] = {{ {fields} }};\n\
             static const NS_Gc_Shape {{NAME}} = {{ .kind = NS_SHAPE_KIND_RECORD, \
             .fields = {{NAME}}_f, .field_count = {count} }};"
        );
        self.define_shape(&template)
    }

    fn variant_shape(&mut self, c_name: &str, variants: &[String]) -> String {
        if variants.iter().all(|entry| entry == "NULL") {
            return "&NS_SHAPE_LEAF".to_owned();
        }
        let template = format!(
            "static const NS_Gc_Shape* {{NAME}}_v[] = {{ {} }};\n\
             static const NS_Gc_Shape {{NAME}} = {{ .kind = NS_SHAPE_KIND_VARIANT, \
             .tag_offset = offsetof({c_name}, tag), .variants = {{NAME}}_v, \
             .variant_count = {} }};",
            variants.join(", "),
            variants.len()
        );
        self.define_shape(&template)
    }

    fn define_shape(&mut self, template: &str) -> String {
        if let Some(existing) = self.shape_keys.get(template) {
            return format!("&{existing}");
        }
        let name = format!("NS_SHAPE_{}", self.shapes.len());
        self.shapes.push(template.replace("{NAME}", &name));
        self.shape_keys.insert(template.to_owned(), name.clone());
        format!("&{name}")
    }

    pub fn root_exprs(&mut self, base: &str, ty: &Type) -> Vec<String> {
        if let Type::Generic(name) = ty
            && let Some(concrete) = self.subst.get(name).cloned()
        {
            return self.root_exprs(base, &concrete);
        }
        if let Type::Named(name, arguments) = ty
            && arguments.is_empty()
            && let Some(concrete) = self.subst.get(name).cloned()
            && concrete != *ty
        {
            return self.root_exprs(base, &concrete);
        }
        match ty {
            Type::Str | Type::Bytes => vec![format!("(void*)(({base}).data)")],
            Type::Array(_) | Type::Slice(_) => vec![format!("(void*)(({base}).data)")],
            Type::Map(_, _) => vec![
                format!("(void*)(({base}).keys)"),
                format!("(void*)(({base}).values)"),
            ],
            Type::Option(inner) => self
                .root_exprs(&format!("({base}).value"), inner)
                .into_iter()
                .map(|slot| format!("(({base}).some ? {slot} : NULL)"))
                .collect(),
            Type::Result(ok, error) => {
                let mut slots = self
                    .root_exprs(&format!("({base}).value"), ok)
                    .into_iter()
                    .map(|slot| format!("(({base}).ok ? {slot} : NULL)"))
                    .collect::<Vec<_>>();
                slots.extend(
                    self.root_exprs(&format!("({base}).error"), error)
                        .into_iter()
                        .map(|slot| format!("(({base}).ok ? NULL : {slot})")),
                );
                slots
            }
            Type::Named(name, args) => match name.as_str() {
                "Ptr" | "Region" => Vec::new(),

                "Channel" => vec![format!("(void*)({base})")],
                "Task" => vec![format!("(void*)({base})")],
                "SliceError" | "ParseIntError" | "OverflowError" | "TaskError" => {
                    vec![format!("(void*)(({base})._0.data)")]
                }
                "ChannelError" => Vec::new(),
                _ => {
                    if !args.is_empty() {
                        return Vec::new();
                    }
                    match self.env.shape(name).cloned() {
                        Some(DeclShape::Struct(shape)) => {
                            let mut slots = Vec::new();
                            for (field, field_ty) in &shape.fields {
                                slots.extend(
                                    self.root_exprs(&format!("({base}).{field}"), field_ty),
                                );
                            }
                            slots
                        }
                        Some(DeclShape::Enum(shape)) => {
                            let mut slots = Vec::new();
                            for variant in &shape.variants {
                                if variant.fields.is_empty() {
                                    continue;
                                }
                                let Some(tag) = self.enum_tag(name, &variant.name) else {
                                    continue;
                                };
                                let mut variant_slots = Vec::new();
                                if variant.fields.len() == 1 {
                                    variant_slots.extend(self.root_exprs(
                                        &format!("({base}).payload.{}", variant.name),
                                        &variant.fields[0],
                                    ));
                                } else {
                                    for (index, field_ty) in variant.fields.iter().enumerate() {
                                        variant_slots.extend(self.root_exprs(
                                            &format!("({base}).payload.{}._{index}", variant.name),
                                            field_ty,
                                        ));
                                    }
                                }
                                slots.extend(variant_slots.into_iter().map(|slot| {
                                    format!("(({base}).tag == {tag} ? {slot} : NULL)")
                                }));
                            }
                            slots
                        }
                        _ => Vec::new(),
                    }
                }
            },
            _ => Vec::new(),
        }
    }

    pub fn root_slot_count(&mut self, ty: &Type) -> usize {
        self.root_exprs("_", ty).len()
    }

    fn user_typedef(&mut self, name: &str) -> String {
        let cname = format!("NS_{}{}", self.type_prefix, sanitize(name));

        if let Some(nominal) = self.nominal(name)
            && nominal.foreign
        {
            return format!("NS_{}{}", nominal.prefix, sanitize(&nominal.source));
        }

        let owned = match self.env.shape(name) {
            None => {
                if let Some(builtin) = builtin_nominal(name) {
                    return format!("NS_{builtin}");
                }
                self.rejects.push(Reject {
                    code: "NS8020",
                    message: format!("type '{name}' is not declared in this module"),
                });
                return cname;
            }
            Some(DeclShape::Struct(s)) => OwnedDecl::Struct(s.fields.clone()),
            Some(DeclShape::Enum(e)) => OwnedDecl::Enum(e.variants.clone()),
        };
        match owned {
            OwnedDecl::Struct(fields) => {
                let mut fields_c = String::new();
                for (field_name, field_type) in &fields {
                    let field_c = self.c_type(field_type);
                    fields_c.push_str(&format!("{field_c} {field_name}; "));
                }
                self.ensure(&cname, format!("typedef struct {{ {fields_c}}} {cname};"));
            }
            OwnedDecl::Enum(variants) => {
                let tag_name = format!("NS_{}{}_tag", self.type_prefix, sanitize(name));
                let mut tags = String::new();
                for (index, variant) in variants.iter().enumerate() {
                    tags.push_str(&format!(
                        "NS_{}{}_TAG_{} = {index}, ",
                        self.type_prefix,
                        sanitize(name),
                        variant.name
                    ));
                }
                self.ensure(&tag_name, format!("typedef enum {{ {tags}}} {tag_name};"));
                let mut members = String::new();
                for variant in &variants {
                    if variant.fields.is_empty() {
                        members.push_str(&format!("char _{name}; ", name = variant.name));
                    } else if variant.fields.len() == 1 {
                        let field_c = self.c_type(&variant.fields[0]);
                        members.push_str(&format!("{field_c} {name}; ", name = variant.name));
                    } else {
                        let mut inner = String::new();
                        for (index, field_type) in variant.fields.iter().enumerate() {
                            let field_c = self.c_type(field_type);
                            inner.push_str(&format!("{field_c} _{index}; "));
                        }
                        members.push_str(&format!(
                            "struct {{ {inner}}} {name}; ",
                            name = variant.name
                        ));
                    }
                }
                self.ensure(
                    &cname,
                    format!(
                        "typedef struct {{ {tag_name} tag; union {{ {members}}} payload; }} {cname};"
                    ),
                );
            }
        }
        cname
    }
}

#[derive(Debug, Clone, Default)]
pub struct Annotations {
    by_span: BTreeMap<(usize, usize), Type>,
}

impl Annotations {
    pub fn new(annotations: &[astra_core::checker::TypeAnnotation]) -> Self {
        let mut by_span = BTreeMap::new();
        for ann in annotations {
            by_span.insert((ann.span.start, ann.span.end), ann.ty.clone());
        }
        Self { by_span }
    }

    pub fn type_of(&self, expr: &ast::Expr) -> Option<&Type> {
        self.by_span.get(&(expr.span.start, expr.span.end))
    }

    #[allow(dead_code)]
    pub fn at(&self, span: Span) -> Option<&Type> {
        self.by_span.get(&(span.start, span.end))
    }
}
