use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::rc::Rc;

use astra_core::ast;
use astra_core::checker::CheckResult;
use astra_core::hir;
use astra_core::mode::Mode;
use astra_core::span::Span;
use astra_core::types::Type;

use crate::error::{Abort, Fault, Frame, NovaError, Panic};
use crate::host::Host;
use crate::value::{
    BuiltinFunction, ChannelValue, Constructor, ConstructorKind, Float, FunctionValue, Integer,
    StructFieldsValue, Task, TaskFailure, Value, values_equal,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum IntegerPolicy {

    #[default]
    Wrapping,

    SafeChecked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NovaOptions {

    pub mode: Mode,

    pub infer_mode: bool,
    pub integer_policy: IntegerPolicy,
    pub max_call_depth: usize,
}

impl Default for NovaOptions {
    fn default() -> Self {
        Self {
            mode: Mode::Core,
            infer_mode: true,
            integer_policy: IntegerPolicy::Wrapping,
            max_call_depth: 1024,
        }
    }
}

type EnvRef = Rc<RefCell<Scope>>;

#[derive(Default)]
struct Scope {
    parent: Option<EnvRef>,
    values: BTreeMap<String, Value>,
    defers: Vec<ast::Stmt>,
    tasks: Vec<Rc<RefCell<Task>>>,
}

impl Scope {
    fn child(parent: &EnvRef) -> EnvRef {
        Rc::new(RefCell::new(Self {
            parent: Some(parent.clone()),
            ..Self::default()
        }))
    }

    fn global() -> EnvRef {
        Rc::new(RefCell::new(Self::default()))
    }
}

enum Flow {
    Value(Value),
    Return(Value),
    Failure(Value),
    Break,
    Continue,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ArithmeticMode {
    Normal,
    Wrapping,
    Checked,
}

pub struct Interpreter {
    options: NovaOptions,
    host: Box<dyn Host>,
    modules: BTreeMap<String, RuntimeModule>,
    active: String,
    fault_module: Option<String>,
    stack: Vec<Frame>,
}

struct RuntimeModule {
    module: hir::Module,
    globals: EnvRef,
    annotations: HashMap<Span, Type>,
    functions: BTreeMap<String, Rc<FunctionValue>>,
    constants: BTreeMap<String, Rc<ast::ConstantDecl>>,
    constant_values: HashMap<String, Value>,
    constants_evaluating: HashSet<String>,
    structs: BTreeMap<String, Rc<ast::StructDecl>>,
    enums: BTreeMap<String, Rc<ast::EnumDecl>>,
    foreign_modules: BTreeSet<String>,
    imported_values: BTreeMap<String, (String, String)>,
    imported_namespaces: BTreeMap<String, String>,
}

const SINGLE_MODULE: &str = "<entry>";

fn runtime_module(id: &str, checked: CheckResult) -> Result<RuntimeModule, NovaError> {
    if let Some(span) = checked.manual_memory_scopes.first() {
        return Err(NovaError::ManualMemoryUnsupported { span: *span });
    }

    let mut annotations = HashMap::new();
    for annotation in checked.annotations {
        annotations.entry(annotation.span).or_insert(annotation.ty);
    }

    let mut functions = BTreeMap::new();
    let mut constants = BTreeMap::new();
    let mut structs = BTreeMap::new();
    let mut enums = BTreeMap::new();
    let mut foreign_modules = BTreeSet::new();
    let mut scripts = BTreeMap::new();

    for item in &checked.module.items {
        match &item.declaration {
            hir::Declaration::Function(declaration) => {
                functions.insert(
                    declaration.name.clone(),
                    Rc::new(FunctionValue::new(id, Rc::new(declaration.clone()))),
                );
            }
            hir::Declaration::Constant(declaration) => {
                constants.insert(declaration.name.clone(), Rc::new(declaration.clone()));
            }
            hir::Declaration::Struct(declaration) => {
                structs.insert(declaration.name.clone(), Rc::new(declaration.clone()));
            }
            hir::Declaration::Enum(declaration) => {
                enums.insert(declaration.name.clone(), Rc::new(declaration.clone()));
            }
            hir::Declaration::ForeignModule(declaration) => {
                foreign_modules.insert(declaration.name.clone());
            }
            hir::Declaration::ScriptBlock(script) => {
                scripts.insert(script.name.clone(), (script.language.clone(), script.span));
            }
            hir::Declaration::TypeAlias(_) => {}
        }
    }

    let mut imported_values = BTreeMap::new();
    let mut imported_namespaces = BTreeMap::new();
    for import in &checked.module.imports {
        match &import.kind {
            hir::ImportKind::SideEffect => {}
            hir::ImportKind::Namespace { alias } => {
                imported_namespaces.insert(alias.clone(), import.module.clone());
            }
            hir::ImportKind::Named { bindings } => {
                for binding in bindings {
                    imported_values.insert(
                        binding.local.clone(),
                        (import.module.clone(), binding.imported.clone()),
                    );
                }
            }
            hir::ImportKind::Default { local } => {
                imported_values
                    .insert(local.clone(), (import.module.clone(), "default".to_owned()));
            }
            hir::ImportKind::DefaultNamed { default, bindings } => {
                imported_values.insert(
                    default.clone(),
                    (import.module.clone(), "default".to_owned()),
                );
                for binding in bindings {
                    imported_values.insert(
                        binding.local.clone(),
                        (import.module.clone(), binding.imported.clone()),
                    );
                }
            }
        }
    }

    let globals = Scope::global();
    for (name, function) in &functions {
        globals
            .borrow_mut()
            .values
            .insert(name.clone(), Value::Function(function.clone()));
    }
    for (name, declaration) in &structs {
        if let ast::StructFields::Tuple(fields) = &declaration.fields
            && !fields.is_empty()
        {
            globals.borrow_mut().values.insert(
                name.clone(),
                Value::Constructor(Constructor {
                    kind: ConstructorKind::TupleStruct {
                        module: id.to_owned(),
                        name: name.clone(),
                    },
                }),
            );
        }
    }
    for name in &foreign_modules {
        globals
            .borrow_mut()
            .values
            .insert(name.clone(), Value::ForeignModule(name.clone()));
    }
    for (name, (language, span)) in scripts {
        globals.borrow_mut().values.insert(
            name.clone(),
            Value::ScriptBlock {
                name,
                language,
                span,
            },
        );
    }

    Ok(RuntimeModule {
        module: checked.module,
        globals,
        annotations,
        functions,
        constants,
        constant_values: HashMap::new(),
        constants_evaluating: HashSet::new(),
        structs,
        enums,
        foreign_modules,
        imported_values,
        imported_namespaces,
    })
}

macro_rules! eval_value {
    ($expression:expr) => {
        match $expression? {
            Flow::Value(value) => value,
            flow => return Ok(flow),
        }
    };
}

impl Interpreter {

    pub fn new(
        checked: CheckResult,
        options: NovaOptions,
        host: Box<dyn Host>,
    ) -> Result<Self, NovaError> {
        let module = runtime_module(SINGLE_MODULE, checked)?;
        let mut modules = BTreeMap::new();
        modules.insert(SINGLE_MODULE.to_owned(), module);
        Ok(Self {
            options,
            host,
            modules,
            active: SINGLE_MODULE.to_owned(),
            fault_module: None,
            stack: Vec::new(),
        })
    }

    pub(crate) fn from_modules(
        entry: impl Into<String>,
        checked_modules: Vec<(String, CheckResult)>,
        options: NovaOptions,
        host: Box<dyn Host>,
    ) -> Result<Self, NovaError> {
        let entry = entry.into();
        let mut modules = BTreeMap::new();
        for (id, checked) in checked_modules {
            if let Some(span) = checked.manual_memory_scopes.first() {
                return Err(NovaError::ManualMemoryUnsupported { span: *span });
            }
            modules.insert(id.clone(), runtime_module(&id, checked)?);
        }
        if !modules.contains_key(&entry) {
            return Err(NovaError::MissingEntry {
                name: entry.clone(),
            });
        }
        Ok(Self {
            options,
            host,
            modules,
            active: entry,
            fault_module: None,
            stack: Vec::new(),
        })
    }

    pub fn run(&mut self) -> Result<Value, NovaError> {
        let value = self.call("main", Vec::new())?;
        match value {
            Value::Failure(error) => Err(NovaError::UnhandledFailure(*error)),
            value => Ok(value),
        }
    }

    pub fn call(&mut self, name: &str, arguments: Vec<Value>) -> Result<Value, NovaError> {
        self.fault_module = None;
        let Some(function) = self.current().functions.get(name).cloned() else {
            return Err(NovaError::MissingEntry {
                name: name.to_owned(),
            });
        };
        self.invoke_user_function(&function, arguments)
            .map_err(Fault::into_error)
    }

    #[must_use]
    pub fn host(&self) -> &dyn Host {
        self.host.as_ref()
    }

    #[must_use]
    pub fn host_mut(&mut self) -> &mut dyn Host {
        self.host.as_mut()
    }

    #[must_use]
    pub fn module_name(&self) -> &str {
        &self.current().module.name
    }

    #[must_use]
    pub fn active_module(&self) -> &str {
        &self.active
    }

    #[must_use]
    pub fn fault_module(&self) -> Option<&str> {
        self.fault_module.as_deref()
    }

    fn current(&self) -> &RuntimeModule {
        self.modules
            .get(&self.active)
            .expect("active module must exist")
    }

    fn current_mut(&mut self) -> &mut RuntimeModule {
        self.modules
            .get_mut(&self.active)
            .expect("active module must exist")
    }

    fn invoke_user_function(
        &mut self,
        function: &Rc<FunctionValue>,
        arguments: Vec<Value>,
    ) -> Result<Value, Fault> {
        let declaration = function.declaration.clone();
        if self.stack.len() >= self.options.max_call_depth {
            return Err(self.panic(
                format!(
                    "call stack exceeded {} frames while calling '{}'",
                    self.options.max_call_depth, declaration.name
                ),
                declaration.span,
            ));
        }
        if arguments.len() != declaration.parameters.len() {
            return Err(self.internal(format!(
                "function '{}' expected {} argument(s), found {} after type checking",
                declaration.name,
                declaration.parameters.len(),
                arguments.len()
            )));
        }

        let previous = std::mem::replace(&mut self.active, function.module.clone());
        let frame = if previous == SINGLE_MODULE && function.module == SINGLE_MODULE {
            Frame::new(declaration.name.clone(), declaration.span)
        } else {
            Frame::in_module(
                function.module.clone(),
                declaration.name.clone(),
                declaration.span,
            )
        };
        self.stack.push(frame);
        let globals = self.current().globals.clone();
        let call_scope = Scope::child(&globals);
        for (parameter, argument) in declaration.parameters.iter().zip(arguments) {
            call_scope
                .borrow_mut()
                .values
                .insert(parameter.name.clone(), argument);
        }

        let evaluated = self.eval_block(&declaration.body, &call_scope);
        self.stack.pop();
        if evaluated.is_err() && self.fault_module.is_none() {
            self.fault_module = Some(self.active.clone());
        }
        self.active = previous;

        match evaluated {
            Ok(Flow::Value(value) | Flow::Return(value)) => {
                if !declaration.error_set.is_empty() && !returns_result(&declaration.return_type) {
                    Ok(Value::Result(Ok(Box::new(value))))
                } else {
                    Ok(value)
                }
            }
            Ok(Flow::Failure(error)) => Ok(Value::Failure(Box::new(error))),
            Ok(Flow::Break | Flow::Continue) => Err(self.internal(format!(
                "loop control escaped function '{}'",
                declaration.name
            ))),
            Err(fault) => Err(fault),
        }
    }

    fn eval_block(&mut self, block: &ast::Block, parent: &EnvRef) -> Result<Flow, Fault> {
        let scope = Scope::child(parent);
        let result = self.eval_statements(&block.statements, &scope);
        self.finish_scope(&scope, result)
    }

    fn eval_statements(&mut self, statements: &[ast::Stmt], scope: &EnvRef) -> Result<Flow, Fault> {
        let mut last = Flow::Value(Value::Unit);
        for statement in statements {
            match self.eval_statement(statement, scope)? {
                Flow::Value(value) => last = Flow::Value(value),
                flow => return Ok(flow),
            }
        }
        Ok(last)
    }

    fn eval_statement(&mut self, statement: &ast::Stmt, scope: &EnvRef) -> Result<Flow, Fault> {
        match &statement.kind {
            ast::StmtKind::Let { name, value, .. } => {
                let value = eval_value!(self.eval_expr(value, scope));
                scope.borrow_mut().values.insert(name.clone(), value);
                Ok(Flow::Value(Value::Unit))
            }
            ast::StmtKind::Return { value } => {
                let value = match value {
                    Some(expression) => eval_value!(self.eval_expr(expression, scope)),
                    None => Value::Unit,
                };
                Ok(Flow::Return(value))
            }
            ast::StmtKind::Break => Ok(Flow::Break),
            ast::StmtKind::Continue => Ok(Flow::Continue),
            ast::StmtKind::Defer(deferred) => {
                scope.borrow_mut().defers.push((**deferred).clone());
                Ok(Flow::Value(Value::Unit))
            }
            ast::StmtKind::While { condition, body } => loop {
                let condition = eval_value!(self.eval_expr(condition, scope));
                let Value::Bool(condition) = condition else {
                    return Err(self.internal("while condition was not a bool"));
                };
                if !condition {
                    return Ok(Flow::Value(Value::Unit));
                }
                match self.eval_block(body, scope)? {
                    Flow::Value(_) | Flow::Continue => {}
                    Flow::Break => return Ok(Flow::Value(Value::Unit)),
                    flow => return Ok(flow),
                }
            },
            ast::StmtKind::For {
                binding,
                iterable,
                body,
            } => {
                let iterable = eval_value!(self.eval_expr(iterable, scope));
                let values = self.iterate(iterable)?;
                for value in values {
                    let iteration = Scope::child(scope);
                    iteration.borrow_mut().values.insert(binding.clone(), value);
                    match self.eval_block(body, &iteration)? {
                        Flow::Value(_) | Flow::Continue => {}
                        Flow::Break => return Ok(Flow::Value(Value::Unit)),
                        flow => return Ok(flow),
                    }
                }
                Ok(Flow::Value(Value::Unit))
            }
            ast::StmtKind::Expr(expression) => self.eval_expr(expression, scope),
            ast::StmtKind::Scoped(scoped) => {
                if scoped.annotation.kind == ast::ScopeKind::Memory
                    && scoped.annotation.value == "manual"
                {
                    return Err(Fault::Error(NovaError::ManualMemoryUnsupported {
                        span: scoped.annotation.span,
                    }));
                }
                let scoped_scope = Scope::child(scope);
                let result = self.eval_statements(&scoped.statements, &scoped_scope);
                let result = self.finish_scope(&scoped_scope, result)?;
                Ok(match result {
                    Flow::Value(_) => Flow::Value(Value::Unit),
                    flow => flow,
                })
            }
        }
    }

    fn finish_scope(&mut self, scope: &EnvRef, result: Result<Flow, Fault>) -> Result<Flow, Fault> {
        if matches!(result, Err(Fault::Abort(_))) {
            return result;
        }

        let mut pending = None;
        loop {
            let tasks = std::mem::take(&mut scope.borrow_mut().tasks);
            for task in tasks {
                let (joined, cancelled, outcome) = {
                    let mut task = task.borrow_mut();
                    let joined = task.joined;
                    task.joined = true;
                    (joined, task.cancelled, task.outcome.clone())
                };
                if !joined {
                    match outcome {
                        Some(Err(TaskFailure::Panic(panic))) => {
                            pending = Some(
                                self.task_error(&panic.message, panic.span.unwrap_or_default()),
                            );
                        }
                        None if cancelled => {
                            pending = Some(self.task_error("task was cancelled", Span::default()));
                        }
                        _ => {}
                    }
                }
            }

            let defers = std::mem::take(&mut scope.borrow_mut().defers);
            if defers.is_empty() {
                break;
            }
            for deferred in defers.into_iter().rev() {
                if let Err(fault) = self.eval_statement(&deferred, scope) {
                    pending = Some(fault);
                    break;
                }
            }
        }

        if let Some(fault) = pending {
            Err(fault)
        } else {
            result
        }
    }

    fn eval_expr(&mut self, expression: &ast::Expr, scope: &EnvRef) -> Result<Flow, Fault> {
        match &expression.kind {
            ast::ExprKind::Literal(literal) => {
                Ok(Flow::Value(self.literal(literal, expression.span)))
            }
            ast::ExprKind::Identifier(name) => {
                let value = self.lookup_value(name, expression.span, scope)?;
                Ok(Flow::Value(value))
            }
            ast::ExprKind::Unary { operator, operand } => {
                let operand = eval_value!(self.eval_expr(operand, scope));
                Ok(Flow::Value(self.unary(
                    *operator,
                    operand,
                    expression.span,
                )?))
            }
            ast::ExprKind::Binary {
                operator,
                left,
                right,
            } => self.eval_binary(*operator, left, right, expression.span, scope),
            ast::ExprKind::Assign { target, value } => {
                let value = eval_value!(self.eval_expr(value, scope));
                self.assign(target, value, expression.span, scope)
            }
            ast::ExprKind::Call { callee, arguments } => {
                if let ast::ExprKind::Identifier(name) = &callee.kind
                    && let Some(kind) = builtin_constructor_kind(name)
                {
                    let mut values = Vec::with_capacity(arguments.len());
                    for argument in arguments {
                        values.push(eval_value!(self.eval_expr(argument, scope)));
                    }
                    return Ok(Flow::Value(self.call_constructor(
                        kind,
                        values,
                        expression.span,
                    )?));
                }
                let callee = eval_value!(self.eval_expr(callee, scope));
                let mut values = Vec::with_capacity(arguments.len());
                for argument in arguments {
                    values.push(eval_value!(self.eval_expr(argument, scope)));
                }
                Ok(Flow::Value(self.call_value(
                    callee,
                    values,
                    expression.span,
                )?))
            }
            ast::ExprKind::Index { target, index } => {
                let target = eval_value!(self.eval_expr(target, scope));
                let index = eval_value!(self.eval_expr(index, scope));
                Ok(Flow::Value(self.index(target, index, expression.span)?))
            }
            ast::ExprKind::Member { target, member } => {
                self.member(target, member, expression.span, scope)
            }
            ast::ExprKind::TypeArguments { target, .. } => self.eval_expr(target, scope),
            ast::ExprKind::Propagate(inner) => {
                let value = eval_value!(self.eval_expr(inner, scope));
                match value {
                    Value::Failure(error) => Ok(Flow::Failure(*error)),
                    Value::Result(Err(error)) => Ok(Flow::Failure(*error)),
                    Value::Result(Ok(value)) => Ok(Flow::Value(*value)),
                    value => Ok(Flow::Value(value)),
                }
            }
            ast::ExprKind::Array(elements) => {
                let mut values = Vec::with_capacity(elements.len());
                for element in elements {
                    values.push(eval_value!(self.eval_expr(element, scope)));
                }
                Ok(Flow::Value(Value::array(values)))
            }
            ast::ExprKind::Map(entries) => {
                let mut map = Vec::with_capacity(entries.len());
                for entry in entries {
                    let key = eval_value!(self.eval_expr(&entry.key, scope));
                    let value = eval_value!(self.eval_expr(&entry.value, scope));
                    map_insert(&mut map, key, value);
                }
                Ok(Flow::Value(Value::map(map)))
            }
            ast::ExprKind::Tuple(elements) => {
                let mut values = Vec::with_capacity(elements.len());
                for element in elements {
                    values.push(eval_value!(self.eval_expr(element, scope)));
                }
                Ok(Flow::Value(Value::Tuple(Rc::new(RefCell::new(values)))))
            }
            ast::ExprKind::StructInit { path, fields } => {
                self.struct_init(path, fields, expression.span, scope)
            }
            ast::ExprKind::Block(block) => self.eval_block(block, scope),
            ast::ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let condition = eval_value!(self.eval_expr(condition, scope));
                let Value::Bool(condition) = condition else {
                    return Err(self.internal("if condition was not a bool"));
                };
                if condition {
                    self.eval_block(then_branch, scope)
                } else if let Some(else_branch) = else_branch {
                    match else_branch {
                        ast::ElseBranch::Block(block) => self.eval_block(block, scope),
                        ast::ElseBranch::If(expression) => self.eval_expr(expression, scope),
                    }
                } else {
                    Ok(Flow::Value(Value::Unit))
                }
            }
            ast::ExprKind::Match { value, arms } => {
                let value = match self.eval_expr(value, scope)? {
                    Flow::Value(value) | Flow::Failure(value) => value,
                    flow => return Ok(flow),
                };
                self.eval_match(value, arms, expression.span, scope)
            }
            ast::ExprKind::Spawn { body } => self.eval_spawn(body, expression.span, scope),
            ast::ExprKind::Fail { error, arguments } => {
                self.failure_value(error, arguments, expression.span, scope)
            }
            ast::ExprKind::Panic { message } => {
                let message = eval_value!(self.eval_expr(message, scope));
                let Value::Str(message) = message else {
                    return Err(self.internal("panic message was not a str"));
                };
                Err(self.panic((*message).to_owned(), expression.span))
            }
            ast::ExprKind::Abort { message } => {
                let message = eval_value!(self.eval_expr(message, scope));
                let Value::Str(message) = message else {
                    return Err(self.internal("abort message was not a str"));
                };
                Err(Fault::Abort(Abort {
                    message: (*message).to_owned(),
                    span: Some(expression.span),
                }))
            }
        }
    }

    fn eval_binary(
        &mut self,
        operator: ast::BinaryOp,
        left: &ast::Expr,
        right: &ast::Expr,
        span: Span,
        scope: &EnvRef,
    ) -> Result<Flow, Fault> {
        if matches!(operator, ast::BinaryOp::And | ast::BinaryOp::Or) {
            let left = eval_value!(self.eval_expr(left, scope));
            let Value::Bool(left) = left else {
                return Err(self.internal("logical operand was not a bool"));
            };
            if (operator == ast::BinaryOp::And && !left) || (operator == ast::BinaryOp::Or && left)
            {
                return Ok(Flow::Value(Value::Bool(left)));
            }
            let right = eval_value!(self.eval_expr(right, scope));
            let Value::Bool(right) = right else {
                return Err(self.internal("logical operand was not a bool"));
            };
            return Ok(Flow::Value(Value::Bool(right)));
        }

        let left = eval_value!(self.eval_expr(left, scope));
        let right = eval_value!(self.eval_expr(right, scope));

        let value = match operator {
            ast::BinaryOp::Equal => Value::Bool(values_equal(&left, &right)),
            ast::BinaryOp::NotEqual => Value::Bool(!values_equal(&left, &right)),
            ast::BinaryOp::Less
            | ast::BinaryOp::LessEqual
            | ast::BinaryOp::Greater
            | ast::BinaryOp::GreaterEqual => Value::Bool(self.compare(operator, &left, &right)?),
            ast::BinaryOp::Add
            | ast::BinaryOp::Subtract
            | ast::BinaryOp::Multiply
            | ast::BinaryOp::Divide
            | ast::BinaryOp::Remainder
            | ast::BinaryOp::AddWrap
            | ast::BinaryOp::SubtractWrap
            | ast::BinaryOp::MultiplyWrap
            | ast::BinaryOp::DivideWrap => self.numeric_binary(operator, left, right, span)?,
            ast::BinaryOp::And | ast::BinaryOp::Or => unreachable!("handled above"),
        };
        Ok(Flow::Value(value))
    }

    fn eval_match(
        &mut self,
        value: Value,
        arms: &[ast::MatchArm],
        span: Span,
        scope: &EnvRef,
    ) -> Result<Flow, Fault> {
        let value = match value {
            Value::Failure(error) => Value::Result(Err(error)),
            value => value,
        };

        for arm in arms {
            let arm_scope = Scope::child(scope);
            if self.match_pattern(&arm.pattern, &value, &arm_scope) {
                let result = match &arm.body {
                    ast::MatchArmBody::Expr(expression) => self.eval_expr(expression, &arm_scope),
                    ast::MatchArmBody::Block(block) => self.eval_block(block, &arm_scope),
                };
                return self.finish_scope(&arm_scope, result);
            }
        }

        Err(self.panic(
            format!("non-exhaustive match reached at runtime for {value}"),
            span,
        ))
    }

    fn call_value(
        &mut self,
        callee: Value,
        arguments: Vec<Value>,
        span: Span,
    ) -> Result<Value, Fault> {
        match callee {
            Value::Function(function) => self.invoke_user_function(&function, arguments),
            Value::Builtin(function) => self.call_builtin(function, arguments, span),
            Value::Constructor(constructor) => {
                self.call_constructor(constructor.kind, arguments, span)
            }
            Value::BoundMethod { receiver, name } => {
                self.call_method(*receiver, &name, arguments, span)
            }
            Value::ForeignModule(name) => Err(Fault::Error(NovaError::Unsupported {
                message: format!(
                    "Nova cannot call foreign module '{name}' without a host adapter."
                ),
                span: Some(span),
            })),
            Value::ScriptBlock {
                name,
                language,
                span: script_span,
            } => Err(Fault::Error(NovaError::ForeignScriptUnsupported {
                name,
                language,
                span: script_span,
            })),
            Value::ScriptFunction {
                script_name,
                language,
                span: script_span,
                ..
            } => Err(Fault::Error(NovaError::ForeignScriptUnsupported {
                name: script_name,
                language,
                span: script_span,
            })),
            Value::Imported { module, name } => {
                if !self.modules.contains_key(&module) {
                    return Err(Fault::Error(NovaError::Unsupported {
                        message: format!(
                            "Nova does not execute imported value '{module}::{name}' without a linked project module."
                        ),
                        span: Some(span),
                    }));
                }
                let value = self.export_value(&module, &name, span)?.ok_or_else(|| {
                    self.internal(format!(
                        "imported value '{module}::{name}' has no runtime export"
                    ))
                })?;
                self.call_value(value, arguments, span)
            }
            Value::Failure(error) => Err(self.internal(format!(
                "attempted to call a failure value after type checking: {error}"
            ))),
            callee => Err(self.internal(format!(
                "attempted to call non-callable value after type checking: {callee}"
            ))),
        }
    }

    fn call_builtin(
        &mut self,
        function: BuiltinFunction,
        arguments: Vec<Value>,
        span: Span,
    ) -> Result<Value, Fault> {
        match function {
            BuiltinFunction::Print | BuiltinFunction::Println | BuiltinFunction::Dbg => {
                let [value] = arguments.as_slice() else {
                    return Err(
                        self.internal("console builtin argument count changed after checking")
                    );
                };
                match function {
                    BuiltinFunction::Print => self.host.stdout(&value.render()),
                    BuiltinFunction::Println => {
                        let mut line = value.render();
                        line.push('\n');
                        self.host.stdout(&line);
                    }
                    BuiltinFunction::Dbg => {
                        let mut line = value.render_debug();
                        line.push('\n');
                        self.host.stderr(&line);
                    }
                    _ => unreachable!(),
                }
                Ok(Value::Unit)
            }
            BuiltinFunction::Assert => {
                let [value] = arguments.as_slice() else {
                    return Err(self.internal("assert argument count changed after checking"));
                };
                let Value::Bool(value) = value else {
                    return Err(self.internal("assert argument was not a bool"));
                };
                if *value {
                    Ok(Value::Unit)
                } else {
                    Err(self.panic("assertion failed", span))
                }
            }
        }
    }

    fn call_constructor(
        &mut self,
        kind: ConstructorKind,
        arguments: Vec<Value>,
        span: Span,
    ) -> Result<Value, Fault> {
        match kind {
            ConstructorKind::Some => {
                let [value] = arguments.as_slice() else {
                    return Err(
                        self.internal("Some constructor argument count changed after checking")
                    );
                };
                Ok(Value::Option(Some(Box::new(value.clone()))))
            }
            ConstructorKind::Ok => {
                let [value] = arguments.as_slice() else {
                    return Err(
                        self.internal("Ok constructor argument count changed after checking")
                    );
                };
                Ok(Value::Result(Ok(Box::new(value.clone()))))
            }
            ConstructorKind::Err => {
                let [value] = arguments.as_slice() else {
                    return Err(
                        self.internal("Err constructor argument count changed after checking")
                    );
                };
                Ok(Value::Result(Err(Box::new(value.clone()))))
            }
            ConstructorKind::BytesNew => {
                expect_arguments("bytes.new", &arguments, 1)?;
                let Value::Integer(size) = &arguments[0] else {
                    return Err(self.internal("bytes.new length was not an integer after checking"));
                };
                let limit = usize::try_from(isize::MAX).unwrap_or(usize::MAX);
                let Some(length) = usize::try_from(size.value)
                    .ok()
                    .filter(|length| *length <= limit)
                else {
                    return Err(self.panic(
                        format!("byte buffer length '{}' is out of range", size.value),
                        span,
                    ));
                };
                Ok(Value::Bytes(Rc::new(RefCell::new(vec![0; length]))))
            }
            ConstructorKind::ChannelCreate => {
                expect_arguments("Channel.create", &arguments, 0)?;
                Ok(Value::Channel(Rc::new(RefCell::new(ChannelValue::new()))))
            }
            ConstructorKind::TupleStruct { name, .. } => Ok(Value::tuple_struct(name, arguments)),
            ConstructorKind::EnumVariant {
                enum_name, variant, ..
            } => Ok(Value::enum_value(enum_name, variant, arguments)),
        }
    }

    fn call_method(
        &mut self,
        receiver: Value,
        name: &str,
        arguments: Vec<Value>,
        span: Span,
    ) -> Result<Value, Fault> {
        match (&receiver, name) {
            (Value::Str(value), "len") => {
                expect_arguments(name, &arguments, 0)?;
                Ok(Value::Integer(Integer::new(
                    i128::try_from(value.len()).unwrap_or(i128::MAX),
                    Type::Usize,
                )))
            }
            (Value::Str(value), "is_empty") => {
                expect_arguments(name, &arguments, 0)?;
                Ok(Value::Bool(value.is_empty()))
            }
            (Value::Str(value), "to_int") => {
                expect_arguments(name, &arguments, 0)?;
                match value.parse::<i32>() {
                    Ok(value) => Ok(Value::Result(Ok(Box::new(Value::Integer(Integer::new(
                        i128::from(value),
                        Type::I32,
                    )))))),
                    Err(_) => Ok(Value::Result(Err(Box::new(Value::tuple_struct(
                        "ParseIntError",
                        vec![Value::string(value.to_string())],
                    ))))),
                }
            }
            (Value::Str(value), "char_count") => {
                expect_arguments(name, &arguments, 0)?;
                Ok(Value::Integer(Integer::new(
                    i128::try_from(value.chars().count()).unwrap_or(i128::MAX),
                    Type::Usize,
                )))
            }
            (Value::Str(value), "bytes") => {
                expect_arguments(name, &arguments, 0)?;
                Ok(Value::array(
                    value
                        .as_bytes()
                        .iter()
                        .map(|byte| Value::Integer(Integer::new(i128::from(*byte), Type::U8)))
                        .collect(),
                ))
            }
            (Value::Str(value), "chars") => {
                expect_arguments(name, &arguments, 0)?;
                Ok(Value::array(
                    value
                        .chars()
                        .map(|character| Value::string(character.to_string()))
                        .collect(),
                ))
            }
            (Value::Str(value), "byte_at") => {
                let index = expect_argument(name, &arguments, 0)?;
                let Some(index) = checked_offset(index) else {
                    return Ok(Value::Option(None));
                };
                Ok(Value::Option(value.as_bytes().get(index).copied().map(
                    |byte| Box::new(Value::Integer(Integer::new(i128::from(byte), Type::U8))),
                )))
            }
            (Value::Str(value), "char_at") => {
                let index = expect_argument(name, &arguments, 0)?;
                let Some(index) = checked_offset(index) else {
                    return Ok(Value::Option(None));
                };
                Ok(Value::Option(value.chars().nth(index).map(|character| {
                    Box::new(Value::string(character.to_string()))
                })))
            }
            (Value::Str(value), "slice") => {
                let arguments = expect_arguments(name, &arguments, 2)?;
                let (Some(start), Some(end)) =
                    (checked_offset(&arguments[0]), checked_offset(&arguments[1]))
                else {
                    return Ok(slice_error());
                };
                let bytes = value.as_bytes();
                let boundary = |position: usize| {
                    position == bytes.len()
                        || (position < bytes.len() && (bytes[position] & 0xC0) != 0x80)
                };
                if start > end || end > bytes.len() || !boundary(start) || !boundary(end) {
                    return Ok(slice_error());
                }
                match std::str::from_utf8(&bytes[start..end]) {
                    Ok(part) => Ok(Value::Result(Ok(Box::new(Value::string(part))))),
                    Err(_) => Ok(slice_error()),
                }
            }
            (Value::Bytes(bytes), "len") => {
                expect_arguments(name, &arguments, 0)?;
                Ok(Value::Integer(Integer::new(
                    i128::try_from(bytes.borrow().len()).unwrap_or(i128::MAX),
                    Type::Usize,
                )))
            }
            (Value::Bytes(bytes), "is_empty") => {
                expect_arguments(name, &arguments, 0)?;
                Ok(Value::Bool(bytes.borrow().is_empty()))
            }
            (Value::Bytes(bytes), "slice") => {
                let arguments = expect_arguments(name, &arguments, 2)?;
                let (Some(start), Some(end)) =
                    (checked_offset(&arguments[0]), checked_offset(&arguments[1]))
                else {
                    return Err(self.panic("bytes slice index is out of range", span));
                };
                let bytes = bytes.borrow();
                if start > end || end > bytes.len() {
                    return Err(
                        self.panic(format!("bytes slice {start}..{end} is out of bounds"), span)
                    );
                }
                Ok(Value::Bytes(Rc::new(RefCell::new(
                    bytes[start..end].to_vec(),
                ))))
            }
            (Value::Array(values), "len") => {
                expect_arguments(name, &arguments, 0)?;
                Ok(Value::Integer(Integer::new(
                    i128::try_from(values.borrow().len()).unwrap_or(i128::MAX),
                    Type::Usize,
                )))
            }
            (Value::Array(values), "push") => {
                let value = expect_argument(name, &arguments, 0)?;
                values.borrow_mut().push(value.clone());
                Ok(Value::Unit)
            }
            (Value::Array(values), "get") => {
                let index = expect_argument(name, &arguments, 0)?;
                let index = self.index_usize(index, span)?;
                Ok(Value::Option(
                    values.borrow().get(index).cloned().map(Box::new),
                ))
            }
            (Value::Map(entries), "len") => {
                expect_arguments(name, &arguments, 0)?;
                Ok(Value::Integer(Integer::new(
                    i128::try_from(entries.borrow().len()).unwrap_or(i128::MAX),
                    Type::Usize,
                )))
            }
            (Value::Map(entries), "get") => {
                let key = expect_argument(name, &arguments, 0)?;
                let value = entries
                    .borrow()
                    .iter()
                    .find(|(candidate, _)| values_equal(candidate, key))
                    .map(|(_, value)| value.clone());
                Ok(Value::Option(value.map(Box::new)))
            }
            (Value::Map(entries), "insert") => {
                let arguments = expect_arguments(name, &arguments, 2)?;
                let key = arguments[0].clone();
                let value = arguments[1].clone();
                let previous = map_insert(&mut entries.borrow_mut(), key, value);
                Ok(Value::Option(previous.map(Box::new)))
            }
            (Value::Option(value), "is_some") => {
                expect_arguments(name, &arguments, 0)?;
                Ok(Value::Bool(value.is_some()))
            }
            (Value::Option(value), "is_none") => {
                expect_arguments(name, &arguments, 0)?;
                Ok(Value::Bool(value.is_none()))
            }
            (Value::Option(value), "unwrap") => {
                expect_arguments(name, &arguments, 0)?;
                value
                    .clone()
                    .map(|value| *value)
                    .ok_or_else(|| self.panic("called unwrap() on None", span))
            }
            (Value::Option(value), "unwrap_or") => {
                let fallback = expect_argument(name, &arguments, 0)?;
                Ok(value
                    .clone()
                    .map_or_else(|| fallback.clone(), |value| *value))
            }
            (Value::Result(value), "is_ok") => {
                expect_arguments(name, &arguments, 0)?;
                Ok(Value::Bool(value.is_ok()))
            }
            (Value::Result(value), "is_err") => {
                expect_arguments(name, &arguments, 0)?;
                Ok(Value::Bool(value.is_err()))
            }
            (Value::Result(value), "unwrap") => {
                expect_arguments(name, &arguments, 0)?;
                match value.clone() {
                    Ok(value) => Ok(*value),
                    Err(error) => Err(self.panic(format!("called unwrap() on Err({error})"), span)),
                }
            }
            (Value::Result(value), "unwrap_or") => {
                let fallback = expect_argument(name, &arguments, 0)?;
                Ok(match value.clone() {
                    Ok(value) => *value,
                    Err(_) => fallback.clone(),
                })
            }
            (Value::Result(value), "error") => {
                expect_arguments(name, &arguments, 0)?;
                match value.clone() {
                    Err(error) => Ok(*error),
                    Ok(value) => Err(self.panic(format!("called error() on Ok({value})"), span)),
                }
            }
            (Value::Task(task), "join") => {
                expect_arguments(name, &arguments, 0)?;
                let (cancelled, outcome) = {
                    let mut task = task.borrow_mut();
                    task.joined = true;
                    (task.cancelled, task.outcome.clone())
                };
                match outcome {
                    Some(Ok(value)) => Ok(value),
                    Some(Err(TaskFailure::Error(error))) => Ok(Value::Failure(Box::new(error))),
                    Some(Err(TaskFailure::Panic(panic))) => {
                        Err(self.task_error(&panic.message, span))
                    }
                    None if cancelled => Err(self.task_error("task was cancelled", span)),
                    None => Err(self.internal("joined a task without an outcome")),
                }
            }
            (Value::Task(task), "cancel") => {
                expect_arguments(name, &arguments, 0)?;
                let mut task = task.borrow_mut();

                if task.outcome.is_none() {
                    task.cancelled = true;
                }
                Ok(Value::Unit)
            }
            (Value::Task(task), "canceled") => {
                expect_arguments(name, &arguments, 0)?;
                Ok(Value::Bool(task.borrow().cancelled))
            }
            (Value::Channel(channel), "send") => {
                let value = expect_argument(name, &arguments, 0)?.clone();
                let mut channel = channel.borrow_mut();
                if channel.closed {
                    return Ok(channel_error("send on a closed channel"));
                }
                if channel.buffer.len() >= channel.capacity {
                    return Err(self.panic(
                        "channel deadlock: send on a full bounded channel would block \
                         (single-threaded runtime)",
                        span,
                    ));
                }
                channel.buffer.push(value);
                Ok(Value::Result(Ok(Box::new(Value::Unit))))
            }
            (Value::Channel(channel), "receive") => {
                expect_arguments(name, &arguments, 0)?;
                let mut channel = channel.borrow_mut();
                if channel.buffer.is_empty() {
                    if channel.closed {
                        return Ok(channel_error("receive on a closed channel"));
                    }
                    return Err(self.panic(
                        "channel deadlock: receive on an empty channel would block \
                         (single-threaded runtime)",
                        span,
                    ));
                }
                let value = channel.buffer.remove(0);
                Ok(Value::Result(Ok(Box::new(value))))
            }
            (Value::Channel(channel), "close") => {
                expect_arguments(name, &arguments, 0)?;
                channel.borrow_mut().closed = true;
                Ok(Value::Unit)
            }
            (Value::Integer(integer), method) => {
                self.integer_method(integer.clone(), method, &arguments, span)
            }
            _ => Err(self.internal(format!(
                "method '{name}' was not implemented for receiver {receiver} after type checking"
            ))),
        }
    }

    fn integer_method(
        &mut self,
        integer: Integer,
        method: &str,
        arguments: &[Value],
        span: Span,
    ) -> Result<Value, Fault> {
        let right = expect_argument(method, arguments, 0)?;
        let Value::Integer(right) = right else {
            return Err(self.internal("integer method argument was not an integer"));
        };
        let right = right.value;

        let wrapping = match method {
            "wrapping_add" => Some(ast::BinaryOp::Add),
            "wrapping_sub" => Some(ast::BinaryOp::Subtract),
            "wrapping_mul" => Some(ast::BinaryOp::Multiply),
            "wrapping_div" => Some(ast::BinaryOp::Divide),
            "checked_add" => Some(ast::BinaryOp::Add),
            "checked_sub" => Some(ast::BinaryOp::Subtract),
            "checked_mul" => Some(ast::BinaryOp::Multiply),
            "checked_div" => Some(ast::BinaryOp::Divide),
            _ => None,
        };
        let Some(operator) = wrapping else {
            return Err(self.internal(format!("unknown integer method '{method}'")));
        };
        let checked = method.starts_with("checked_");
        let mode = if method.starts_with("wrapping_") {
            ArithmeticMode::Wrapping
        } else if checked {
            ArithmeticMode::Checked
        } else {
            ArithmeticMode::Normal
        };
        let result = self.integer_operation(
            operator,
            integer.value,
            right,
            integer.ty.clone(),
            mode,
            span,
        )?;
        if let Some(value) = result {
            if checked {
                return Ok(Value::Result(Ok(Box::new(Value::Integer(Integer::new(
                    value, integer.ty,
                ))))));
            }
            return Ok(Value::Integer(Integer::new(value, integer.ty)));
        }
        if checked {
            Ok(Value::Result(Err(Box::new(Value::tuple_struct(
                "OverflowError",
                vec![Value::string(format!("integer overflow in '{method}'"))],
            )))))
        } else {
            Err(self.panic(
                format!("integer overflow in '{method}' on type '{}'", integer.ty),
                span,
            ))
        }
    }

    fn struct_init(
        &mut self,
        path: &ast::TypePath,
        fields: &[ast::FieldInit],
        span: Span,
        scope: &EnvRef,
    ) -> Result<Flow, Fault> {
        let name = path
            .segments
            .last()
            .cloned()
            .unwrap_or_else(|| path.segments.join("."));

        let current_struct = self.current().structs.get(&name).cloned();
        let definition = if let Some(declaration) = current_struct {
            Some(declaration)
        } else {
            let imported = {
                let current = self.current();
                if let Some(module) = path
                    .segments
                    .first()
                    .filter(|_| path.segments.len() > 1)
                    .and_then(|segment| current.imported_namespaces.get(segment))
                {
                    Some((module.clone(), name.clone()))
                } else {
                    current
                        .imported_values
                        .get(&name)
                        .map(|(module, imported)| (module.clone(), imported.clone()))
                }
            };
            match imported {
                Some((module, imported)) => self.resolve_struct(&module, &imported)?,
                None => None,
            }
        };

        let Some(definition) = definition else {
            let namespace = path.segments.first().filter(|segment| {
                let current = self.current();
                current.imported_namespaces.contains_key(*segment)
                    || current.foreign_modules.contains(*segment)
            });
            let imported = self.current().imported_values.contains_key(&name);
            if imported || namespace.is_some() {
                return Err(Fault::Error(NovaError::Unsupported {
                    message: format!(
                        "Nova cannot construct imported type '{}' because it is not a linked project struct.",
                        path.segments.join(".")
                    ),
                    span: Some(span),
                }));
            }
            return Err(self.internal(format!(
                "unknown struct '{name}' reached the interpreter after type checking"
            )));
        };

        let mut values = Vec::with_capacity(fields.len());
        for field in fields {
            let value = eval_value!(self.eval_expr(&field.value, scope));
            values.push((field.name.clone(), value));
        }

        let _ = definition;
        Ok(Flow::Value(Value::named_struct(name, values)))
    }

    fn resolve_struct(
        &mut self,
        module: &str,
        name: &str,
    ) -> Result<Option<Rc<ast::StructDecl>>, Fault> {
        self.resolve_struct_inner(module, name, &mut BTreeSet::new())
    }

    fn resolve_struct_inner(
        &mut self,
        module: &str,
        name: &str,
        seen: &mut BTreeSet<(String, String)>,
    ) -> Result<Option<Rc<ast::StructDecl>>, Fault> {
        if !seen.insert((module.to_owned(), name.to_owned())) {
            return Err(self.internal(format!(
                "cyclic export chain while resolving struct '{module}::{name}'"
            )));
        }
        let Some(export) = self.export(module, name) else {
            return Ok(None);
        };
        match export.target {
            hir::ExportTarget::Local(local) => Ok(self
                .modules
                .get(module)
                .and_then(|state| state.structs.get(&local))
                .cloned()),
            hir::ExportTarget::Reexport { module, name } => {
                self.resolve_struct_inner(&module, &name, seen)
            }
            hir::ExportTarget::Namespace { .. } | hir::ExportTarget::Expression => Ok(None),
        }
    }

    fn member(
        &mut self,
        target: &ast::Expr,
        member: &str,
        span: Span,
        scope: &EnvRef,
    ) -> Result<Flow, Fault> {
        if let ast::ExprKind::Identifier(name) = &target.kind {
            let is_local_enum = {
                let current = self.current();
                self.scope_lookup(scope, name).is_none()
                    && !current.constants.contains_key(name)
                    && !current.imported_values.contains_key(name)
                    && !current.imported_namespaces.contains_key(name)
                    && !current.globals.borrow().values.contains_key(name)
                    && current.enums.contains_key(name)
            };
            if is_local_enum {
                let declaration = self.current().enums.get(name).cloned().expect("checked");
                return self.enum_member(self.active.clone(), name, &declaration, member);
            }

            let is_builtin_namespace = {
                let current = self.current();
                self.scope_lookup(scope, name).is_none()
                    && !current.constants.contains_key(name)
                    && !current.imported_values.contains_key(name)
                    && !current.imported_namespaces.contains_key(name)
                    && !current.globals.borrow().values.contains_key(name)
                    && !current.structs.contains_key(name)
            };
            if is_builtin_namespace {
                let kind = match (name.as_str(), member) {
                    ("bytes", "new") => Some(ConstructorKind::BytesNew),
                    ("Channel", "create") => Some(ConstructorKind::ChannelCreate),
                    _ => None,
                };
                if let Some(kind) = kind {
                    return Ok(Flow::Value(Value::Constructor(Constructor { kind })));
                }
            }
        }

        let value = eval_value!(self.eval_expr(target, scope));
        let result: Result<Value, Fault> = match &value {
            Value::Struct(instance) => {
                let instance = instance.borrow();
                match &instance.fields {
                    StructFieldsValue::Named(fields) => fields
                        .iter()
                        .find(|(field, _)| field == member)
                        .map(|(_, value)| value.clone())
                        .ok_or_else(|| {
                            self.internal(format!(
                                "struct '{}' has no field '{member}' after type checking",
                                instance.name
                            ))
                        }),
                    StructFieldsValue::Tuple(fields) => {
                        let index = member.parse::<usize>().map_err(|_| {
                            self.internal(format!(
                                "tuple struct '{}' member '{member}' is not an index",
                                instance.name
                            ))
                        })?;
                        fields.get(index).cloned().ok_or_else(|| {
                            self.internal(format!(
                                "tuple struct '{}' member '{member}' is out of bounds after type checking",
                                instance.name
                            ))
                        })
                    }
                }
            }
            Value::Tuple(elements) => {
                let index = member.parse::<usize>().map_err(|_| {
                    self.internal(format!("tuple member '{member}' is not an index"))
                })?;
                elements.borrow().get(index).cloned().ok_or_else(|| {
                    self.internal(format!(
                        "tuple member '{member}' is out of bounds after type checking"
                    ))
                })
            }
            Value::ModuleNamespace(module) => {
                self.export_value(module, member, span)?.ok_or_else(|| {
                    self.internal(format!(
                        "module '{module}' has no runtime export '{member}'"
                    ))
                })
            }
            Value::EnumNamespace { module, enum_name } => {
                let declaration = self
                    .modules
                    .get(module)
                    .and_then(|state| state.enums.get(enum_name))
                    .cloned()
                    .ok_or_else(|| {
                        self.internal(format!(
                            "enum '{module}::{enum_name}' disappeared after type checking"
                        ))
                    })?;
                self.enum_member(module.clone(), enum_name, &declaration, member)
                    .map(|flow| match flow {
                        Flow::Value(value) => value,
                        _ => Value::Unit,
                    })
            }
            Value::ForeignModule(module) => Ok(Value::Imported {
                module: module.clone(),
                name: member.to_owned(),
            }),
            Value::ScriptBlock {
                name,
                language,
                span,
            } => Ok(Value::ScriptFunction {
                script_name: name.clone(),
                function_name: member.to_owned(),
                language: language.clone(),
                span: *span,
            }),
            Value::Imported { module, .. } => Ok(Value::Imported {
                module: module.clone(),
                name: member.to_owned(),
            }),
            _ => Ok(Value::BoundMethod {
                receiver: Box::new(value.clone()),
                name: member.to_owned(),
            }),
        };
        result.map(Flow::Value).map_err(|fault| match fault {
            Fault::Error(NovaError::Internal(message)) => Fault::Error(NovaError::Unsupported {
                message,
                span: Some(span),
            }),
            fault => fault,
        })
    }

    fn enum_member(
        &mut self,
        module: String,
        enum_name: &str,
        declaration: &Rc<ast::EnumDecl>,
        member: &str,
    ) -> Result<Flow, Fault> {
        let Some(variant) = declaration
            .variants
            .iter()
            .find(|variant| variant.name == member)
        else {
            return Err(self.internal(format!(
                "enum '{enum_name}' has no variant '{member}' after type checking"
            )));
        };
        if variant.fields.is_none() {
            return Ok(Flow::Value(Value::enum_value(
                enum_name,
                member,
                Vec::new(),
            )));
        }
        Ok(Flow::Value(Value::Constructor(Constructor {
            kind: ConstructorKind::EnumVariant {
                module,
                enum_name: enum_name.to_owned(),
                variant: member.to_owned(),
            },
        })))
    }

    fn export(&self, module: &str, name: &str) -> Option<hir::Export> {
        let state = self.modules.get(module)?;
        if name == "default" {
            state.module.exports.default.clone()
        } else {
            state.module.exports.get_named(name).cloned()
        }
    }

    fn export_value(
        &mut self,
        module: &str,
        name: &str,
        span: Span,
    ) -> Result<Option<Value>, Fault> {
        self.export_value_inner(module, name, span, &mut BTreeSet::new())
    }

    fn export_value_inner(
        &mut self,
        module: &str,
        name: &str,
        span: Span,
        seen: &mut BTreeSet<(String, String)>,
    ) -> Result<Option<Value>, Fault> {
        if !seen.insert((module.to_owned(), name.to_owned())) {
            return Err(self.internal(format!(
                "cyclic export chain while resolving '{module}::{name}'"
            )));
        }
        let Some(export) = self.export(module, name) else {
            return Ok(None);
        };
        match export.target {
            hir::ExportTarget::Local(local) => self.local_value(module, &local, span),
            hir::ExportTarget::Reexport { module, name } => {
                self.export_value_inner(&module, &name, span, seen)
            }
            hir::ExportTarget::Namespace { module } => Ok(Some(Value::ModuleNamespace(module))),
            hir::ExportTarget::Expression => Err(Fault::Error(NovaError::Unsupported {
                message: format!(
                    "Nova cannot evaluate the anonymous default expression exported by '{module}'."
                ),
                span: Some(span),
            })),
        }
    }

    fn local_value(
        &mut self,
        module: &str,
        name: &str,
        span: Span,
    ) -> Result<Option<Value>, Fault> {
        let Some(state) = self.modules.get(module) else {
            return Ok(None);
        };

        if let Some(function) = state.functions.get(name) {
            return Ok(Some(Value::Function(function.clone())));
        }
        if let Some(declaration) = state.constants.get(name).cloned() {
            let previous = std::mem::replace(&mut self.active, module.to_owned());
            let result = self.evaluate_constant(name, &declaration, span);
            if result.is_err() && self.fault_module.is_none() {
                self.fault_module = Some(module.to_owned());
            }
            self.active = previous;
            return result.map(Some);
        }
        if let Some(declaration) = state.structs.get(name).cloned() {
            if let ast::StructFields::Tuple(fields) = &declaration.fields
                && !fields.is_empty()
            {
                return Ok(Some(Value::Constructor(Constructor {
                    kind: ConstructorKind::TupleStruct {
                        module: module.to_owned(),
                        name: declaration.name.clone(),
                    },
                })));
            }
            return Err(Fault::Error(NovaError::Unsupported {
                message: format!("Nova cannot use struct '{module}::{name}' as a runtime value."),
                span: Some(span),
            }));
        }
        if state.enums.contains_key(name) {
            return Ok(Some(Value::EnumNamespace {
                module: module.to_owned(),
                enum_name: name.to_owned(),
            }));
        }
        if state.foreign_modules.contains(name) {
            return Ok(Some(Value::ForeignModule(name.to_owned())));
        }
        if let Some((target, imported)) = state.imported_values.get(name).cloned() {
            return self.export_value_inner(&target, &imported, span, &mut BTreeSet::new());
        }
        Ok(None)
    }

    fn index(&mut self, target: Value, index: Value, span: Span) -> Result<Value, Fault> {
        match target {
            Value::Array(values) => {
                let index = self.index_usize(&index, span)?;
                values.borrow().get(index).cloned().ok_or_else(|| {
                    self.panic(format!("array index {index} is out of bounds"), span)
                })
            }
            Value::Map(entries) => entries
                .borrow()
                .iter()
                .find(|(key, _)| values_equal(key, &index))
                .map(|(_, value)| value.clone())
                .ok_or_else(|| self.panic("map key is not present", span)),
            Value::Tuple(elements) => {
                let index = self.index_usize(&index, span)?;
                elements.borrow().get(index).cloned().ok_or_else(|| {
                    self.panic(format!("tuple index {index} is out of bounds"), span)
                })
            }
            Value::Bytes(bytes) => {
                let index = self.index_usize(&index, span)?;
                bytes
                    .borrow()
                    .get(index)
                    .copied()
                    .map(|byte| Value::Integer(Integer::new(i128::from(byte), Type::U8)))
                    .ok_or_else(|| self.panic(format!("byte index {index} is out of bounds"), span))
            }
            value => Err(self.internal(format!(
                "indexed non-indexable value after type checking: {value}"
            ))),
        }
    }

    fn assign(
        &mut self,
        target: &ast::Expr,
        value: Value,
        span: Span,
        scope: &EnvRef,
    ) -> Result<Flow, Fault> {
        match &target.kind {
            ast::ExprKind::Identifier(name) => {
                if self.scope_assign(scope, name, value) {
                    Ok(Flow::Value(Value::Unit))
                } else {
                    Err(self.internal(format!(
                        "assignment target '{name}' was not found after type checking"
                    )))
                }
            }
            ast::ExprKind::Member { target, member } => {
                let target = eval_value!(self.eval_expr(target, scope));
                match target {
                    Value::Struct(instance) => {
                        let mut instance = instance.borrow_mut();
                        match &mut instance.fields {
                            StructFieldsValue::Named(fields) => {
                                let Some((_, field)) =
                                    fields.iter_mut().find(|(name, _)| name == member)
                                else {
                                    return Err(self.internal(format!(
                                        "struct '{}' has no field '{member}' after type checking",
                                        instance.name
                                    )));
                                };
                                *field = value;
                                Ok(Flow::Value(Value::Unit))
                            }
                            StructFieldsValue::Tuple(_) => Err(self.internal(
                                "assigned a named member on a tuple struct after type checking",
                            )),
                        }
                    }
                    Value::Tuple(elements) => {
                        let index = member.parse::<usize>().map_err(|_| {
                            self.internal(format!("tuple member '{member}' is not an index"))
                        })?;
                        let mut elements = elements.borrow_mut();
                        let Some(element) = elements.get_mut(index) else {
                            return Err(self.internal(format!(
                                "tuple member '{member}' is out of bounds after type checking"
                            )));
                        };
                        *element = value;
                        Ok(Flow::Value(Value::Unit))
                    }
                    _ => Err(self.internal(
                        "assigned a member of a non-assignable value after type checking",
                    )),
                }
            }
            ast::ExprKind::Index { target, index } => {
                let target = eval_value!(self.eval_expr(target, scope));
                let index = eval_value!(self.eval_expr(index, scope));
                match target {
                    Value::Array(values) => {
                        let index = self.index_usize(&index, span)?;
                        let mut values = values.borrow_mut();
                        let Some(element) = values.get_mut(index) else {
                            return Err(
                                self.panic(format!("array index {index} is out of bounds"), span)
                            );
                        };
                        *element = value;
                        Ok(Flow::Value(Value::Unit))
                    }
                    Value::Map(entries) => {
                        map_insert(&mut entries.borrow_mut(), index, value);
                        Ok(Flow::Value(Value::Unit))
                    }
                    Value::Tuple(elements) => {
                        let index = self.index_usize(&index, span)?;
                        let mut elements = elements.borrow_mut();
                        let Some(element) = elements.get_mut(index) else {
                            return Err(
                                self.panic(format!("tuple index {index} is out of bounds"), span)
                            );
                        };
                        *element = value;
                        Ok(Flow::Value(Value::Unit))
                    }
                    Value::Bytes(bytes) => {
                        let index = self.index_usize(&index, span)?;
                        let Value::Integer(byte) = value else {
                            return Err(
                                self.internal("assigned a non-integer byte after type checking")
                            );
                        };
                        let mut bytes = bytes.borrow_mut();
                        let Some(slot) = bytes.get_mut(index) else {
                            return Err(
                                self.panic(format!("byte index {index} is out of bounds"), span)
                            );
                        };
                        *slot = byte.value as u8;
                        Ok(Flow::Value(Value::Unit))
                    }
                    _ => Err(self.internal(
                        "assigned an index of a non-assignable value after type checking",
                    )),
                }
            }
            _ => Err(self.internal("assignment target was not an lvalue after type checking")),
        }
    }

    fn iterate(&mut self, value: Value) -> Result<Vec<Value>, Fault> {
        match value {
            Value::Array(values) => Ok(values.borrow().clone()),
            Value::Map(entries) => Ok(entries
                .borrow()
                .iter()
                .map(|(key, _)| key.clone())
                .collect()),
            Value::Str(value) => Ok(value
                .chars()
                .map(|character| Value::string(character.to_string()))
                .collect()),
            Value::Bytes(bytes) => Ok(bytes
                .borrow()
                .iter()
                .map(|byte| Value::Integer(Integer::new(i128::from(*byte), Type::U8)))
                .collect()),
            value => Err(self.internal(format!(
                "iterated a non-iterable value after type checking: {value}"
            ))),
        }
    }

    fn match_pattern(&self, pattern: &ast::Pattern, value: &Value, scope: &EnvRef) -> bool {
        match &pattern.kind {
            ast::PatternKind::Wildcard => true,
            ast::PatternKind::Identifier(name) => {
                if name == "None" && matches!(value, Value::Option(_)) {
                    return matches!(value, Value::Option(None));
                }
                scope
                    .borrow_mut()
                    .values
                    .insert(name.clone(), value.clone());
                true
            }
            ast::PatternKind::Literal(literal) => {
                values_equal(&self.pattern_literal(literal, value), value)
            }
            ast::PatternKind::Tuple(patterns) => {
                let Value::Tuple(values) = value else {
                    return false;
                };
                let values = values.borrow();
                patterns.len() == values.len()
                    && patterns
                        .iter()
                        .zip(values.iter())
                        .all(|(pattern, value)| self.match_pattern(pattern, value, scope))
            }
            ast::PatternKind::Constructor { path, arguments } => {
                let name = path
                    .segments
                    .last()
                    .map_or_else(|| path.segments.join("."), Clone::clone);
                match name.as_str() {
                    "Some" => {
                        let Value::Option(Some(value)) = value else {
                            return false;
                        };
                        self.match_arguments(arguments, std::slice::from_ref(&**value), scope)
                    }
                    "Ok" => {
                        let Value::Result(Ok(value)) = value else {
                            return false;
                        };
                        self.match_arguments(arguments, std::slice::from_ref(&**value), scope)
                    }
                    "Err" => {
                        let Value::Result(Err(value)) = value else {
                            return false;
                        };
                        self.match_arguments(arguments, std::slice::from_ref(&**value), scope)
                    }
                    _ => false,
                }
            }
            ast::PatternKind::Variant {
                path,
                variant,
                arguments,
            } => {
                let enum_name = path
                    .segments
                    .last()
                    .map_or_else(|| path.segments.join("."), Clone::clone);
                if enum_name == "Option" || enum_name == "Result" {
                    let constructor = ast::PatternKind::Constructor {
                        path: ast::TypePath {
                            segments: vec![variant.clone()],
                            arguments: Vec::new(),
                            span: pattern.span,
                        },
                        arguments: arguments.clone().unwrap_or_default(),
                    };
                    let constructor = ast::Pattern {
                        kind: constructor,
                        span: pattern.span,
                    };
                    return self.match_pattern(&constructor, value, scope);
                }
                let Value::Enum(instance) = value else {
                    return false;
                };
                let instance = instance.borrow();
                instance.enum_name == enum_name
                    && instance.variant == *variant
                    && self.match_arguments(
                        arguments.as_deref().unwrap_or(&[]),
                        &instance.fields,
                        scope,
                    )
            }
        }
    }

    fn match_arguments(&self, patterns: &[ast::Pattern], values: &[Value], scope: &EnvRef) -> bool {
        patterns.len() == values.len()
            && patterns
                .iter()
                .zip(values)
                .all(|(pattern, value)| self.match_pattern(pattern, value, scope))
    }

    fn pattern_literal(&self, literal: &ast::Literal, expected: &Value) -> Value {
        match literal {
            ast::Literal::Integer(text) => {
                let (raw, suffix_u8) = strip_u8_suffix(text);
                let ty = match expected {
                    Value::Integer(integer) => integer.ty.clone(),
                    _ if suffix_u8 => Type::U8,
                    _ => Type::I32,
                };
                let value = parse_integer(raw)
                    .and_then(|value| i128::try_from(value).ok())
                    .unwrap_or(0);
                Value::Integer(Integer::new(value, ty))
            }
            ast::Literal::Float(text) => {
                let ty = match expected {
                    Value::Float(float) => float.ty.clone(),
                    _ => Type::F64,
                };
                let value = text.replace('_', "").parse::<f64>().unwrap_or(0.0);
                Value::Float(Float::new(value, ty))
            }
            ast::Literal::String(value) => Value::string(value.clone()),
            ast::Literal::Bytes(value) => Value::Bytes(Rc::new(RefCell::new(value.clone()))),
            ast::Literal::Bool(value) => Value::Bool(*value),
            ast::Literal::Unit => Value::Unit,
        }
    }

    fn failure_value(
        &mut self,
        error: &ast::TypePath,
        arguments: &ast::FailArguments,
        _span: Span,
        scope: &EnvRef,
    ) -> Result<Flow, Fault> {
        let name = error
            .segments
            .last()
            .cloned()
            .unwrap_or_else(|| error.segments.join("."));

        let value = match arguments {
            ast::FailArguments::Tuple(arguments) => {
                let mut values = Vec::with_capacity(arguments.len());
                for argument in arguments {
                    values.push(eval_value!(self.eval_expr(argument, scope)));
                }
                if let Some(declaration) = self.current().structs.get(&name).cloned() {
                    return Ok(Flow::Failure(match &declaration.fields {
                        ast::StructFields::Tuple(_) => Value::tuple_struct(name, values),
                        ast::StructFields::Named(fields) => Value::named_struct(
                            name,
                            fields
                                .iter()
                                .zip(values)
                                .map(|(field, value)| (field.name.clone(), value))
                                .collect(),
                        ),
                    }));
                }
                if let Some(declaration) = self.current().enums.get(&name).cloned()
                    && let Some(variant) = declaration.variants.first()
                {
                    return Ok(Flow::Failure(Value::enum_value(
                        name,
                        variant.name.clone(),
                        values,
                    )));
                }
                Value::tuple_struct(name, values)
            }
            ast::FailArguments::Struct(fields) => {
                let mut values = Vec::with_capacity(fields.len());
                for field in fields {
                    let value = eval_value!(self.eval_expr(&field.value, scope));
                    values.push((field.name.clone(), value));
                }
                Value::named_struct(name, values)
            }
        };
        Ok(Flow::Failure(value))
    }

    fn eval_spawn(&mut self, body: &ast::Block, span: Span, scope: &EnvRef) -> Result<Flow, Fault> {
        self.stack.push(Frame::new("<task>", span));
        let evaluated = self.eval_block(body, scope);
        self.stack.pop();

        let outcome = match evaluated {
            Ok(Flow::Value(value) | Flow::Return(value)) => Ok(value),
            Ok(Flow::Failure(error)) => Err(TaskFailure::Error(error)),
            Ok(Flow::Break | Flow::Continue) => {
                return Err(self.internal("loop control escaped a spawned task"));
            }
            Err(Fault::Panic(panic)) => Err(TaskFailure::Panic(panic)),
            Err(Fault::Abort(abort)) => return Err(Fault::Abort(abort)),
            Err(Fault::Error(error)) => return Err(Fault::Error(error)),
        };

        let task = Rc::new(RefCell::new(Task::complete(outcome)));
        scope.borrow_mut().tasks.push(task.clone());
        Ok(Flow::Value(Value::Task(task)))
    }

    fn lookup_value(&mut self, name: &str, span: Span, scope: &EnvRef) -> Result<Value, Fault> {
        if let Some(value) = self.scope_lookup(scope, name) {
            return Ok(value);
        }
        if let Some(constant) = self.current().constants.get(name).cloned() {
            return self.evaluate_constant(name, &constant, span);
        }
        if name == "None" {
            return Ok(Value::Option(None));
        }
        if let Some((module, imported)) = self.current().imported_values.get(name).cloned() {
            if !self.modules.contains_key(&module) {
                return Ok(Value::Imported {
                    module,
                    name: imported,
                });
            }
            return self.export_value(&module, &imported, span)?.ok_or_else(|| {
                self.internal(format!(
                    "imported value '{module}::{imported}' has no runtime export"
                ))
            });
        }
        if let Some(function) = builtin_function(name) {
            return Ok(Value::Builtin(function));
        }
        if let Some(module) = self.current().imported_namespaces.get(name).cloned() {
            if self.modules.contains_key(&module) {
                return Ok(Value::ModuleNamespace(module));
            }
            return Ok(Value::ForeignModule(module));
        }
        if let Some(module) = self.current().foreign_modules.get(name).cloned() {
            return Ok(Value::ForeignModule(module));
        }
        Err(self.internal(format!(
            "unknown identifier '{name}' reached the interpreter after type checking"
        )))
    }

    fn evaluate_constant(
        &mut self,
        name: &str,
        constant: &Rc<ast::ConstantDecl>,
        _span: Span,
    ) -> Result<Value, Fault> {
        if let Some(value) = self.current().constant_values.get(name) {
            return Ok(value.clone());
        }
        if !self
            .current_mut()
            .constants_evaluating
            .insert(name.to_owned())
        {
            return Err(self.internal(format!(
                "constant '{name}' depends on itself after type checking"
            )));
        }
        let globals = self.current().globals.clone();
        let evaluated = self.eval_expr(&constant.value, &globals);
        self.current_mut().constants_evaluating.remove(name);
        match evaluated {
            Ok(Flow::Value(value) | Flow::Return(value)) => {
                self.current_mut()
                    .constant_values
                    .insert(name.to_owned(), value.clone());
                Ok(value)
            }
            Ok(Flow::Failure(error)) => Ok(Value::Failure(Box::new(error))),
            Ok(Flow::Break | Flow::Continue) => {
                Err(self.internal(format!("control flow escaped constant '{name}'")))
            }
            Err(fault) => Err(fault),
        }
    }

    fn scope_lookup(&self, scope: &EnvRef, name: &str) -> Option<Value> {
        let mut current = Some(scope.clone());
        while let Some(scope) = current {
            let borrowed = scope.borrow();
            if let Some(value) = borrowed.values.get(name) {
                return Some(value.clone());
            }
            current = borrowed.parent.clone();
        }
        None
    }

    fn scope_assign(&self, scope: &EnvRef, name: &str, value: Value) -> bool {
        let mut current = Some(scope.clone());
        while let Some(scope) = current {
            let mut borrowed = scope.borrow_mut();
            if let Some(slot) = borrowed.values.get_mut(name) {
                *slot = value;
                return true;
            }
            current = borrowed.parent.clone();
        }
        false
    }

    fn literal(&self, literal: &ast::Literal, span: Span) -> Value {
        match literal {
            ast::Literal::Integer(text) => {
                let (raw, suffix_u8) = strip_u8_suffix(text);
                let ty = self
                    .current()
                    .annotations
                    .get(&span)
                    .filter(|ty| ty.is_integer())
                    .cloned()
                    .unwrap_or(if suffix_u8 { Type::U8 } else { Type::I32 });
                let value = parse_integer(raw)
                    .and_then(|value| i128::try_from(value).ok())
                    .unwrap_or(0);
                Value::Integer(Integer::new(value, ty))
            }
            ast::Literal::Float(text) => {
                let ty = self
                    .current()
                    .annotations
                    .get(&span)
                    .filter(|ty| ty.is_float())
                    .cloned()
                    .unwrap_or(Type::F64);
                let value = text.replace('_', "").parse::<f64>().unwrap_or(0.0);
                Value::Float(Float::new(value, ty))
            }
            ast::Literal::String(value) => Value::string(value.clone()),
            ast::Literal::Bytes(value) => Value::Bytes(Rc::new(RefCell::new(value.clone()))),
            ast::Literal::Bool(value) => Value::Bool(*value),
            ast::Literal::Unit => Value::Unit,
        }
    }

    fn unary(
        &mut self,
        operator: ast::UnaryOp,
        operand: Value,
        span: Span,
    ) -> Result<Value, Fault> {
        match (operator, operand) {
            (ast::UnaryOp::Not, Value::Bool(value)) => Ok(Value::Bool(!value)),
            (ast::UnaryOp::Negate, Value::Integer(integer)) => {
                let value = if self.options.integer_policy == IntegerPolicy::SafeChecked
                    && integer.value == integer_min(&integer.ty)
                {
                    return Err(
                        self.panic(format!("integer overflow negating '{}'", integer.ty), span)
                    );
                } else {
                    integer.value.wrapping_neg()
                };
                Ok(Value::Integer(Integer::new(
                    normalize_integer(value, &integer.ty),
                    integer.ty,
                )))
            }
            (ast::UnaryOp::Negate, Value::Float(float)) => {
                Ok(Value::Float(Float::new(-float.value, float.ty)))
            }
            (operator, value) => Err(self.internal(format!(
                "invalid unary operation {operator:?} on {value} after type checking"
            ))),
        }
    }

    fn compare(&self, operator: ast::BinaryOp, left: &Value, right: &Value) -> Result<bool, Fault> {
        let ordering = match (left, right) {
            (Value::Integer(left), Value::Integer(right)) => left.value.partial_cmp(&right.value),
            (Value::Float(left), Value::Float(right)) => left.value.partial_cmp(&right.value),
            (Value::Str(left), Value::Str(right)) => Some(left.cmp(right)),
            (Value::Bytes(left), Value::Bytes(right)) => Some(left.borrow().cmp(&right.borrow())),
            _ => {
                return Err(self.internal(format!(
                    "invalid ordered comparison between {left} and {right} after type checking"
                )));
            }
        };
        Ok(match operator {
            ast::BinaryOp::Less => ordering.is_some_and(|ordering| ordering.is_lt()),
            ast::BinaryOp::LessEqual => ordering.is_some_and(|ordering| ordering.is_le()),
            ast::BinaryOp::Greater => ordering.is_some_and(|ordering| ordering.is_gt()),
            ast::BinaryOp::GreaterEqual => ordering.is_some_and(|ordering| ordering.is_ge()),
            _ => unreachable!("compare only handles ordered operators"),
        })
    }

    fn numeric_binary(
        &mut self,
        operator: ast::BinaryOp,
        left: Value,
        right: Value,
        span: Span,
    ) -> Result<Value, Fault> {
        match (left, right) {
            (Value::Integer(left), Value::Integer(right)) => {
                let mode = if matches!(
                    operator,
                    ast::BinaryOp::AddWrap
                        | ast::BinaryOp::SubtractWrap
                        | ast::BinaryOp::MultiplyWrap
                        | ast::BinaryOp::DivideWrap
                ) {
                    ArithmeticMode::Wrapping
                } else {
                    ArithmeticMode::Normal
                };
                let operator = match operator {
                    ast::BinaryOp::AddWrap => ast::BinaryOp::Add,
                    ast::BinaryOp::SubtractWrap => ast::BinaryOp::Subtract,
                    ast::BinaryOp::MultiplyWrap => ast::BinaryOp::Multiply,
                    ast::BinaryOp::DivideWrap => ast::BinaryOp::Divide,
                    operator => operator,
                };
                match self.integer_operation(
                    operator,
                    left.value,
                    right.value,
                    left.ty.clone(),
                    mode,
                    span,
                )? {
                    Some(value) => Ok(Value::Integer(Integer::new(value, left.ty))),
                    None => {
                        Err(self.panic(format!("integer overflow computing '{}'", left.ty), span))
                    }
                }
            }
            (Value::Float(left), Value::Float(right)) => {
                let value = match operator {
                    ast::BinaryOp::Add | ast::BinaryOp::AddWrap => left.value + right.value,
                    ast::BinaryOp::Subtract | ast::BinaryOp::SubtractWrap => {
                        left.value - right.value
                    }
                    ast::BinaryOp::Multiply | ast::BinaryOp::MultiplyWrap => {
                        left.value * right.value
                    }
                    ast::BinaryOp::Divide | ast::BinaryOp::DivideWrap => left.value / right.value,
                    ast::BinaryOp::Remainder => left.value % right.value,
                    _ => {
                        return Err(self.internal(format!(
                            "invalid numeric operator {operator:?} after type checking"
                        )));
                    }
                };
                Ok(Value::Float(Float::new(value, left.ty)))
            }
            (left, right) => Err(self.internal(format!(
                "invalid numeric operands {left} and {right} after type checking"
            ))),
        }
    }

    fn integer_operation(
        &mut self,
        operator: ast::BinaryOp,
        left: i128,
        right: i128,
        ty: Type,
        mode: ArithmeticMode,
        span: Span,
    ) -> Result<Option<i128>, Fault> {
        let wrap = mode == ArithmeticMode::Wrapping
            || (mode == ArithmeticMode::Normal
                && self.options.integer_policy == IntegerPolicy::Wrapping);

        let checked_result = match operator {
            ast::BinaryOp::Add => left.checked_add(right),
            ast::BinaryOp::Subtract => left.checked_sub(right),
            ast::BinaryOp::Multiply => left.checked_mul(right),
            ast::BinaryOp::Divide => {
                if right == 0 {
                    return Err(self.panic("division by zero", span));
                }
                left.checked_div(right)
            }
            ast::BinaryOp::Remainder => {
                if right == 0 {
                    return Err(self.panic("division by zero", span));
                }

                Some(left.checked_rem(right).unwrap_or(0))
            }
            _ => {
                return Err(self.internal(format!("invalid integer operator {operator:?}")));
            }
        };

        if wrap {
            let wrapped = match operator {
                ast::BinaryOp::Add => left.wrapping_add(right),
                ast::BinaryOp::Subtract => left.wrapping_sub(right),
                ast::BinaryOp::Multiply => left.wrapping_mul(right),
                ast::BinaryOp::Divide => left.wrapping_div(right),
                ast::BinaryOp::Remainder => left.wrapping_rem(right),
                _ => unreachable!(),
            };
            return Ok(Some(normalize_integer(wrapped, &ty)));
        }

        let Some(value) = checked_result else {
            return Ok(None);
        };
        if !integer_fits(value, &ty) {
            return Ok(None);
        }
        Ok(Some(value))
    }

    fn index_usize(&self, value: &Value, span: Span) -> Result<usize, Fault> {
        let Value::Integer(integer) = value else {
            return Err(self.internal("index was not an integer after type checking"));
        };
        usize::try_from(integer.value)
            .map_err(|_| self.panic(format!("index '{}' is out of range", integer.value), span))
    }

    fn panic(&self, message: impl Into<String>, span: Span) -> Fault {
        let stack = self.stack.iter().rev().cloned().collect();
        Fault::Panic(Panic::new(message, Some(span), stack))
    }

    fn task_error(&self, message: &str, span: Span) -> Fault {
        let message = if message.starts_with("TaskError: ") {
            message.to_owned()
        } else {
            format!("TaskError: {message}")
        };
        self.panic(message, span)
    }

    fn internal(&self, message: impl Into<String>) -> Fault {
        Fault::Error(NovaError::Internal(message.into()))
    }
}

fn expect_arguments<'a>(
    method: &str,
    arguments: &'a [Value],
    count: usize,
) -> Result<&'a [Value], Fault> {
    if arguments.len() == count {
        Ok(arguments)
    } else {
        Err(Fault::Error(NovaError::Internal(format!(
            "method '{method}' argument count changed after type checking"
        ))))
    }
}

fn expect_argument<'a>(
    method: &str,
    arguments: &'a [Value],
    index: usize,
) -> Result<&'a Value, Fault> {
    expect_arguments(method, arguments, index + 1)?;
    Ok(&arguments[index])
}

fn checked_offset(value: &Value) -> Option<usize> {
    match value {
        Value::Integer(integer) => usize::try_from(integer.value).ok(),
        _ => None,
    }
}

fn slice_error() -> Value {
    Value::Result(Err(Box::new(Value::tuple_struct(
        "SliceError",
        vec![Value::string(
            "slice range is out of bounds or splits a UTF-8 scalar value",
        )],
    ))))
}

fn channel_error(message: &str) -> Value {
    Value::Result(Err(Box::new(Value::tuple_struct(
        "ChannelError",
        vec![Value::string(message)],
    ))))
}

fn map_insert(entries: &mut Vec<(Value, Value)>, key: Value, value: Value) -> Option<Value> {
    if let Some((_, slot)) = entries
        .iter_mut()
        .find(|(candidate, _)| values_equal(candidate, &key))
    {
        return Some(std::mem::replace(slot, value));
    }
    entries.push((key, value));
    None
}

fn returns_result(return_type: &Option<ast::Type>) -> bool {
    let Some(return_type) = return_type else {
        return false;
    };
    let ast::TypeKind::Path(path) = &return_type.kind else {
        return false;
    };
    path.segments
        .last()
        .is_some_and(|segment| segment == "Result")
}

fn builtin_constructor_kind(name: &str) -> Option<ConstructorKind> {
    match name {
        "Some" => Some(ConstructorKind::Some),
        "Ok" => Some(ConstructorKind::Ok),
        "Err" => Some(ConstructorKind::Err),
        _ => None,
    }
}

fn builtin_function(name: &str) -> Option<BuiltinFunction> {
    match name {
        "print" => Some(BuiltinFunction::Print),
        "println" => Some(BuiltinFunction::Println),
        "dbg" => Some(BuiltinFunction::Dbg),
        "assert" => Some(BuiltinFunction::Assert),
        _ => None,
    }
}

fn strip_u8_suffix(text: &str) -> (&str, bool) {
    if let Some(raw) = text.strip_suffix("u8") {
        (raw, true)
    } else {
        (text, false)
    }
}

fn parse_integer(text: &str) -> Option<u128> {
    let cleaned = text.replace('_', "");
    if let Some(hex) = cleaned
        .strip_prefix("0x")
        .or_else(|| cleaned.strip_prefix("0X"))
    {
        u128::from_str_radix(hex, 16).ok()
    } else if let Some(binary) = cleaned
        .strip_prefix("0b")
        .or_else(|| cleaned.strip_prefix("0B"))
    {
        u128::from_str_radix(binary, 2).ok()
    } else {
        cleaned.parse::<u128>().ok()
    }
}

fn integer_bits(ty: &Type) -> Option<u32> {
    match ty {
        Type::I8 | Type::U8 => Some(8),
        Type::I16 | Type::U16 => Some(16),
        Type::I32 | Type::U32 => Some(32),
        Type::I64 | Type::U64 | Type::Isize | Type::Usize => Some(64),
        _ => None,
    }
}

fn integer_signed(ty: &Type) -> bool {
    matches!(
        ty,
        Type::I8 | Type::I16 | Type::I32 | Type::I64 | Type::Isize
    )
}

fn normalize_integer(value: i128, ty: &Type) -> i128 {
    let Some(bits) = integer_bits(ty) else {
        return value;
    };
    let modulus = 1_i128 << bits;
    let raw = value & (modulus - 1);
    if integer_signed(ty) && raw >= (1_i128 << (bits - 1)) {
        raw - modulus
    } else {
        raw
    }
}

fn integer_min(ty: &Type) -> i128 {
    match integer_bits(ty) {
        Some(bits) if integer_signed(ty) => -(1_i128 << (bits - 1)),
        _ => 0,
    }
}

fn integer_fits(value: i128, ty: &Type) -> bool {
    let Some(bits) = integer_bits(ty) else {
        return true;
    };
    if integer_signed(ty) {
        let minimum = -(1_i128 << (bits - 1));
        let maximum = (1_i128 << (bits - 1)) - 1;
        (minimum..=maximum).contains(&value)
    } else {
        (0..=(1_i128 << bits) - 1).contains(&value)
    }
}
