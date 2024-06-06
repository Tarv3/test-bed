use crate::program::{ProgramState, VarFieldId, VarIter};

use super::{expr::StringExpr, process::ProcessInfo};

#[derive(Clone, Debug, PartialEq)]
pub enum OutputMap<T> {
    Print,
    Create(T),
    Append(T),
}

impl<T> OutputMap<T> {
    #[allow(dead_code)]
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> OutputMap<U> {
        match self {
            OutputMap::Print => OutputMap::Print,
            OutputMap::Create(value) => OutputMap::Create(f(value)),
            OutputMap::Append(value) => OutputMap::Append(f(value)),
        }
    }

    pub fn map_ref<U>(&self, f: impl FnOnce(&T) -> U) -> OutputMap<U> {
        match self {
            OutputMap::Print => OutputMap::Print,
            OutputMap::Create(value) => OutputMap::Create(f(value)),
            OutputMap::Append(value) => OutputMap::Append(f(value)),
        }
    }
}

#[derive(Clone, Debug)]
pub enum ArgBuilder {
    String(StringExpr),
    Set(VarFieldId),
}

impl ArgBuilder {
    pub fn evaluate<'a>(&'a self, state: &'a ProgramState) -> impl Iterator<Item = String> + 'a {
        match self {
            ArgBuilder::String(value) => VarIter::Single(value.evaluate(state)),
            ArgBuilder::Set(value) => VarIter::List(state.get_var(value.var).map(move |object| {
                match value.field {
                    Some(field) => object
                        .properties
                        .get(&field)
                        .cloned()
                        .unwrap_or(String::new()),
                    None => object.base.clone(),
                }
            })),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Spawn {
    pub working_dir: Option<StringExpr>,
    pub command: StringExpr,
    pub args: Vec<ArgBuilder>,
    pub stdout: OutputMap<StringExpr>,
    pub stderr: OutputMap<StringExpr>,
}

impl Spawn {
    pub fn evaluate(&self, state: &ProgramState) -> ProcessInfo {
        let command = self.command.evaluate(state);
        let mut process = ProcessInfo::new(command);

        process
            .add_args(self.args.iter().flat_map(|arg| arg.evaluate(state)))
            .set_stdout(self.stdout.map_ref(|value| value.evaluate(state).into()))
            .set_stderr(self.stderr.map_ref(|value| value.evaluate(state).into()));

        if let Some(dir) = &self.working_dir {
            let working_dir = dir.evaluate(state);
            process.set_working_dir(working_dir.into());
        }

        process
    }
}

#[derive(Clone, Debug)]
pub enum Command {
    LimitSpawn(usize),
    Sleep(u64),
    Spawn(Spawn),
    WaitAll(Option<u64>),
}
