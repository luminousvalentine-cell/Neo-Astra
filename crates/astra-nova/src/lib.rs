mod interpreter;
mod project;

pub mod error;
pub mod host;
pub mod value;

pub use error::{Abort, Frame, NovaError, Panic};
pub use host::{BufferHost, Host, StdHost};
pub use interpreter::{IntegerPolicy, Interpreter, NovaOptions};
pub use project::{
    CompiledModule, ModuleId, Project, ProjectDiagnostic, ProjectError, ProjectOptions,
    build_project_interpreter, load_project, project_from_sources, run_project,
};
pub use value::{
    BuiltinFunction, Constructor, EnumValue, Float, FunctionValue, Integer, StructFieldsValue,
    StructValue, Task, Value, values_equal,
};

use astra_capabilities::infer_module_mode;
use astra_core::ast;
use astra_core::checker::{CheckOptions, check_resolved};
use astra_core::parser::parse;
use astra_core::resolver::resolve_module;

pub fn run_source(source: &str, options: NovaOptions) -> Result<Value, NovaError> {
    build_interpreter(source, options, Box::new(StdHost))?.run()
}

pub fn resolve_source_mode(
    source: &str,
    options: NovaOptions,
) -> Result<astra_core::mode::Mode, NovaError> {
    let module = parse(source).map_err(NovaError::Diagnostics)?;
    resolve_module_mode(&module, options)
}

fn resolve_module_mode(
    module: &ast::Module,
    options: NovaOptions,
) -> Result<astra_core::mode::Mode, NovaError> {
    if !options.infer_mode {
        return Ok(options.mode);
    }
    let inference = infer_module_mode(module);
    if !inference.is_ok() {
        return Err(NovaError::Diagnostics(inference.diagnostics));
    }
    Ok(inference.resolved_mode())
}

pub fn build_interpreter(
    source: &str,
    options: NovaOptions,
    host: Box<dyn Host>,
) -> Result<Interpreter, NovaError> {
    let module = parse(source).map_err(NovaError::Diagnostics)?;
    let mode = resolve_module_mode(&module, options)?;
    let resolved = resolve_module(&module).map_err(NovaError::Diagnostics)?;
    let checked = check_resolved(&module, &resolved, CheckOptions::new(mode))
        .map_err(NovaError::Diagnostics)?;
    Interpreter::new(
        checked,
        NovaOptions {
            mode,
            infer_mode: false,
            ..options
        },
        host,
    )
}
