use crate::program::ProgramState;

use super::{expr::StringExpr, process::ProcessInfo};

#[derive(Clone, Debug, PartialEq)]
pub enum OutputMap<T> {
    Print,
    Create(T),
    Append(T),
}

impl<T> OutputMap<T> {
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
pub struct Spawn {
    pub command: StringExpr,
    pub args: Vec<StringExpr>,
    pub stdout: OutputMap<StringExpr>,
    pub stderr: OutputMap<StringExpr>,
}

impl Spawn {
    pub fn evaluate(&self, state: &ProgramState) -> ProcessInfo {
        let command = self.command.evaluate(state);
        let mut process = ProcessInfo::new(command);

        process
            .add_args(self.args.iter().map(|arg| arg.evaluate(state)))
            .set_stdout(self.stdout.map_ref(|value| value.evaluate(state).into()))
            .set_stderr(self.stderr.map_ref(|value| value.evaluate(state).into()));

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
