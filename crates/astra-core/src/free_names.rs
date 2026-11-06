use crate::ast::{
    Block, ElseBranch, Expr, ExprKind, FailArguments, MatchArmBody, Pattern, PatternKind, Stmt,
    StmtKind,
};
use std::collections::HashSet;

pub fn spawn_free_names(body: &Block) -> Vec<String> {
    let mut walker = FreeNames {
        scopes: vec![HashSet::new()],
        free: Vec::new(),
        seen: HashSet::new(),
    };
    walker.block(body);
    walker.free
}

pub fn unwrap_type_arguments(e: &Expr) -> &Expr {
    match &e.kind {
        ExprKind::TypeArguments { target, .. } => unwrap_type_arguments(target),
        _ => e,
    }
}

struct FreeNames {
    scopes: Vec<HashSet<String>>,
    free: Vec<String>,
    seen: HashSet<String>,
}

impl FreeNames {
    fn bound(&self, name: &str) -> bool {
        self.scopes.iter().rev().any(|scope| scope.contains(name))
    }

    fn reference(&mut self, name: &str) {
        if self.bound(name) || self.seen.contains(name) {
            return;
        }
        self.seen.insert(name.to_owned());
        self.free.push(name.to_owned());
    }

    fn bind(&mut self, name: &str) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_owned());
        }
    }

    fn push(&mut self) {
        self.scopes.push(HashSet::new());
    }

    fn pop(&mut self) {
        self.scopes.pop();
    }

    fn scoped_block(&mut self, block: &Block) {
        self.push();
        self.block(block);
        self.pop();
    }

    fn block(&mut self, block: &Block) {
        for stmt in &block.statements {
            self.stmt(stmt);
        }
    }

    fn stmt(&mut self, stmt: &Stmt) {
        match &stmt.kind {
            StmtKind::Let { name, value, .. } => {
                self.expr(value);
                self.bind(name);
            }
            StmtKind::Return { value } => {
                if let Some(value) = value {
                    self.expr(value);
                }
            }
            StmtKind::Break | StmtKind::Continue => {}
            StmtKind::Defer(s) => self.stmt(s),
            StmtKind::While { condition, body } => {
                self.expr(condition);
                self.scoped_block(body);
            }
            StmtKind::For {
                binding,
                iterable,
                body,
            } => {
                self.expr(iterable);
                self.push();
                self.bind(binding);
                self.block(body);
                self.pop();
            }
            StmtKind::Expr(e) => self.expr(e),
            StmtKind::Scoped(s) => {
                self.push();
                for stmt in &s.statements {
                    self.stmt(stmt);
                }
                self.pop();
            }
        }
    }

    fn expr(&mut self, e: &Expr) {
        match &e.kind {
            ExprKind::Literal(_) => {}
            ExprKind::Identifier(name) => self.reference(name),
            ExprKind::Unary { operand, .. } => self.expr(operand),
            ExprKind::Binary { left, right, .. } => {
                self.expr(left);
                self.expr(right);
            }
            ExprKind::Assign { target, value } => {
                self.expr(target);
                self.expr(value);
            }
            ExprKind::Call { callee, arguments } => {
                let callee = unwrap_type_arguments(callee);
                if !matches!(callee.kind, ExprKind::Identifier(_)) {
                    self.expr(callee);
                }
                for argument in arguments {
                    self.expr(argument);
                }
            }
            ExprKind::Index { target, index } => {
                self.expr(target);
                self.expr(index);
            }
            ExprKind::Member { target, .. } => self.expr(target),
            ExprKind::TypeArguments { target, .. } => self.expr(target),
            ExprKind::Propagate(inner) => self.expr(inner),
            ExprKind::Array(items) | ExprKind::Tuple(items) => {
                for item in items {
                    self.expr(item);
                }
            }
            ExprKind::Map(entries) => {
                for entry in entries {
                    self.expr(&entry.key);
                    self.expr(&entry.value);
                }
            }
            ExprKind::StructInit { fields, .. } => {
                for field in fields {
                    self.expr(&field.value);
                }
            }
            ExprKind::Block(block) => self.scoped_block(block),
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.expr(condition);
                self.scoped_block(then_branch);
                match else_branch {
                    Some(ElseBranch::Block(block)) => self.scoped_block(block),
                    Some(ElseBranch::If(inner)) => self.expr(inner),
                    None => {}
                }
            }
            ExprKind::Match { value, arms } => {
                self.expr(value);
                for arm in arms {
                    self.push();
                    self.pattern(&arm.pattern);
                    match &arm.body {
                        MatchArmBody::Expr(e) => self.expr(e),
                        MatchArmBody::Block(block) => self.block(block),
                    }
                    self.pop();
                }
            }
            ExprKind::Spawn { body } => self.scoped_block(body),
            ExprKind::Fail { arguments, .. } => match arguments {
                FailArguments::Tuple(items) => {
                    for item in items {
                        self.expr(item);
                    }
                }
                FailArguments::Struct(fields) => {
                    for field in fields {
                        self.expr(&field.value);
                    }
                }
            },
            ExprKind::Panic { message } | ExprKind::Abort { message } => self.expr(message),
        }
    }

    fn pattern(&mut self, pattern: &Pattern) {
        match &pattern.kind {
            PatternKind::Wildcard | PatternKind::Literal(_) => {}
            PatternKind::Identifier(name) => self.bind(name),
            PatternKind::Tuple(items) => {
                for item in items {
                    self.pattern(item);
                }
            }
            PatternKind::Constructor { arguments, .. } => {
                for argument in arguments {
                    self.pattern(argument);
                }
            }
            PatternKind::Variant { arguments, .. } => {
                if let Some(arguments) = arguments {
                    for argument in arguments {
                        self.pattern(argument);
                    }
                }
            }
        }
    }
}
