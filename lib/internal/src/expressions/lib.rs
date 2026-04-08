use std::collections::BTreeMap;
use std::sync::LazyLock;
use std::time::Duration;

use hive_router_config::traffic_shaping::DurationOrExpression;
use vrl::{
    compiler::{compile as vrl_compile, Program as VrlProgram, TargetValue as VrlTargetValue},
    core::Value as VrlValue,
    path::OwnedSegment,
    prelude::{
        state::RuntimeState as VrlState, Context as VrlContext, Function, TimeZone as VrlTimeZone,
    },
    value::Secrets as VrlSecrets,
};

use crate::expressions::{
    error::{ExpressionCompileError, ExpressionExecutionError},
    functions::env::Env,
    ProgramResolutionError,
};

static VRL_FUNCTIONS: LazyLock<Vec<Box<dyn Function>>> = LazyLock::new(|| {
    let mut funcs = vrl::stdlib::all();
    // Our custom functions:
    funcs.push(Box::new(Env));
    funcs
});
static VRL_TIMEZONE: LazyLock<VrlTimeZone> = LazyLock::new(VrlTimeZone::default);

/// This trait provides a unified way to convert VRL values to specific Rust types.
pub trait FromVrlValue: Sized {
    /// Associated error type for this conversion
    type Error: std::error::Error + Send + Sync + 'static;

    /// Convert a VRL value to this type
    /// - `value` - The VRL value to convert
    fn from_vrl_value(value: VrlValue) -> Result<Self, Self::Error>;
}

/// This trait provides a convenient method to convert sonic_rs Values to VRL Values.
pub trait ToVrlValue {
    /// Convert a sonic_rs Value to a VRL Value
    fn to_vrl_value(&self) -> VrlValue;
}

/// This trait provides a convenient method to compile expressions directly on string types.
pub trait CompileExpression {
    /// Compile a VRL expression string into an executable program
    /// - `functions` - Optional custom functions; if None, uses standard VRL functions
    fn compile_expression(
        &self,
        functions: Option<&[Box<dyn Function>]>,
    ) -> Result<VrlProgram, ExpressionCompileError>;
}

impl CompileExpression for str {
    fn compile_expression(
        &self,
        functions: Option<&[Box<dyn Function>]>,
    ) -> Result<VrlProgram, ExpressionCompileError> {
        let functions = functions.unwrap_or(&VRL_FUNCTIONS);

        let compilation_result = vrl_compile(self, functions).map_err(|diagnostics| {
            ExpressionCompileError::new(
                self.to_string(),
                // Format diagnostics into a human-readable string like this:
                // error[E203]: syntax error
                //   ┌─ :1:23
                //   │
                // 1 │ if (.request.headerss["x-timeout"] == "short") {
                //   │                       ^^^^^^^^^^^
                //   │                       │
                //   │                       unexpected syntax token: "StringLiteral"
                //   │                       expected one of: "integer literal"
                //   │
                //   = see language documentation at https://vrl.dev
                //   = try your code in the VRL REPL, learn more at https://vrl.dev/examples
                vrl::diagnostic::Formatter::new(self, diagnostics).to_string(),
            )
        })?;

        Ok(compilation_result.program)
    }
}

/// Provides a convenient `.execute()` method on VRL `Program` types
/// that handles all the boilerplate of setting up execution context,
/// target values, and error handling.
pub trait ExecutableProgram {
    fn execute(&self, value: VrlValue) -> Result<VrlValue, ExpressionExecutionError>;
}

impl ExecutableProgram for VrlProgram {
    #[inline]
    fn execute(&self, value: VrlValue) -> Result<VrlValue, ExpressionExecutionError> {
        let mut target = VrlTargetValue {
            value,
            metadata: VrlValue::Object(BTreeMap::new()),
            secrets: VrlSecrets::default(),
        };

        let mut state = VrlState::default();
        let mut ctx = VrlContext::new(&mut target, &mut state, &VRL_TIMEZONE);

        Ok(self.resolve(&mut ctx)?)
    }
}

pub enum ValueOrProgram<T> {
    /// A statically-known value
    Value(T),
    /// A VRL program that computes the value at runtime
    Program(Box<VrlProgram>, ProgramHints),
}

impl<T> ValueOrProgram<T>
where
    T: FromVrlValue + Clone,
{
    /// Resolve this ValueOrProgram to a concrete value
    ///
    /// If this is a static value, returns it immediately.
    /// If this is a program, executes it against the provided context and converts the result.
    ///
    /// - `vrl_context_fn` - A function that returns the VRL value context for expression execution
    #[inline]
    pub fn resolve<F>(&self, vrl_context_fn: F) -> Result<T, ProgramResolutionError<T::Error>>
    where
        F: FnOnce() -> VrlValue,
    {
        match self {
            ValueOrProgram::Value(v) => Ok(v.clone()),
            ValueOrProgram::Program(vrl_program, _) => {
                let vrl_context = vrl_context_fn();
                let result_value = vrl_program
                    .execute(vrl_context)
                    .map_err(ProgramResolutionError::ExecutionFailed)?;

                T::from_vrl_value(result_value).map_err(ProgramResolutionError::ConversionFailed)
            }
        }
    }

    /// Resolve this ValueOrProgram to a concrete value, providing target query hints
    /// to the context function so it can optimize the VRL context structure.
    ///
    /// - `vrl_context_fn` - A function that returns the VRL value context, given the query hints
    #[inline]
    pub fn resolve_with_hints<F>(
        &self,
        vrl_context_fn: F,
    ) -> Result<T, ProgramResolutionError<T::Error>>
    where
        F: FnOnce(&ProgramHints) -> VrlValue,
    {
        match self {
            ValueOrProgram::Value(v) => Ok(v.clone()),
            ValueOrProgram::Program(vrl_program, hints) => {
                let vrl_context = vrl_context_fn(hints);
                let result_value = vrl_program
                    .execute(vrl_context)
                    .map_err(ProgramResolutionError::ExecutionFailed)?;

                T::from_vrl_value(result_value).map_err(ProgramResolutionError::ConversionFailed)
            }
        }
    }
}

impl ValueOrProgram<Duration> {
    pub fn compile(
        config: &DurationOrExpression,
        fns: Option<&[Box<dyn Function>]>,
    ) -> Result<Self, ExpressionCompileError> {
        match config {
            DurationOrExpression::Duration(dur) => Ok(ValueOrProgram::Value(*dur)),
            DurationOrExpression::Expression { expression } => {
                let program = expression.as_str().compile_expression(fns)?;
                let hints = ProgramHints::from_program(&program);
                Ok(ValueOrProgram::Program(Box::new(program), hints))
            }
        }
    }
}

#[derive(Debug, Default)]
struct HintNode {
    is_terminal: bool,
    children: Vec<(String, HintNode)>,
}

impl HintNode {
    fn insert(&mut self, path: &[OwnedSegment]) {
        if path.is_empty() {
            self.is_terminal = true;
            return;
        }

        let OwnedSegment::Field(ref f) = path[0] else {
            return; // Ignore index segments
        };
        let key = f.as_str();

        let child_idx = if let Some(idx) = self.children.iter().position(|(k, _)| k == key) {
            idx
        } else {
            self.children.push((key.to_string(), HintNode::default()));
            self.children.len() - 1
        };

        self.children[child_idx].1.insert(&path[1..]);
    }

    fn get_child(&self, key: &str) -> Option<&HintNode> {
        self.children.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
}

/// This struct analyzes a VRL program to determine which variables are accessed
/// during execution.
/// The purpose of this struct is to selectively build context for expressions.
#[derive(Debug, Default)]
pub struct ProgramHints {
    root: HintNode,
}

impl ProgramHints {
    pub fn from_program(program: &VrlProgram) -> Self {
        let mut root = HintNode::default();
        for q in &program.info().target_queries {
            root.insert(&q.path.segments);
        }
        Self { root }
    }

    pub fn context_builder<'a>(
        &'a self,
        build_fn: impl FnOnce(&mut VrlObjectBuilder<'a, '_>),
    ) -> VrlValue {
        VrlContextBuilder::new(self).build_root(build_fn)
    }
}

pub struct VrlContextBuilder<'a> {
    hints: &'a ProgramHints,
}

impl<'a> VrlContextBuilder<'a> {
    fn new(hints: &'a ProgramHints) -> Self {
        Self { hints }
    }

    /// Entry point to build the root object
    fn build_root(&self, build_fn: impl FnOnce(&mut VrlObjectBuilder<'a, '_>)) -> VrlValue {
        let mut map = BTreeMap::new();
        let mut obj_builder = VrlObjectBuilder {
            node: Some(&self.hints.root),
            force_build: self.hints.root.is_terminal,
            map: &mut map,
        };
        build_fn(&mut obj_builder);
        VrlValue::Object(map)
    }
}

enum ChildState<'a> {
    Skip,
    Force,
    Explore(&'a HintNode),
}

/// A builder tied to a specific depth in the object tree.
pub struct VrlObjectBuilder<'a, 'b> {
    node: Option<&'a HintNode>,
    force_build: bool,
    map: &'b mut BTreeMap<vrl::value::KeyString, VrlValue>,
}

impl<'a, 'b> VrlObjectBuilder<'a, 'b> {
    /// Inserts a lazy value if the path is requested.
    pub fn insert_lazy<F>(&mut self, key: &'static str, value_fn: F) -> &mut Self
    where
        F: FnOnce() -> VrlValue,
    {
        if !matches!(self.evaluate_child(key), ChildState::Skip) {
            self.map.insert(key.into(), value_fn());
        }
        self
    }

    /// Nests a new object. The inner builder will be skipped entirely if the parent key
    /// is not requested.
    pub fn insert_object(
        &mut self,
        key: &'static str,
        build_fn: impl FnOnce(&mut VrlObjectBuilder<'a, '_>),
    ) -> &mut Self {
        let child_state = self.evaluate_child(key);

        if matches!(child_state, ChildState::Skip) {
            return self;
        }

        let mut inner_map = BTreeMap::new();

        let force_build = matches!(child_state, ChildState::Force);
        let child_node = match child_state {
            ChildState::Explore(n) => Some(n),
            _ => None,
        };

        let mut sub_builder = VrlObjectBuilder {
            node: child_node,
            force_build,
            map: &mut inner_map,
        };
        build_fn(&mut sub_builder);

        // Only insert the object if children were added or it was explicitly requested
        if !inner_map.is_empty() || force_build {
            self.map.insert(key.into(), VrlValue::Object(inner_map));
        }
        self
    }

    /// Evaluates a key to determine how its children should be built.
    #[inline]
    fn evaluate_child(&self, key: &str) -> ChildState<'a> {
        if self.force_build {
            return ChildState::Force;
        }

        let Some(node) = self.node else {
            return ChildState::Skip;
        };

        let Some(child) = node.get_child(key) else {
            return ChildState::Skip;
        };

        if child.is_terminal {
            return ChildState::Force;
        }

        return ChildState::Explore(child);
    }
}
