use crate::program::{Object, ProgramState, VarFieldId, VariableAccessError};

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

    #[allow(dead_code)]
    pub fn map_ref<U>(&self, f: impl FnOnce(&T) -> U) -> OutputMap<U> {
        match self {
            OutputMap::Print => OutputMap::Print,
            OutputMap::Create(value) => OutputMap::Create(f(value)),
            OutputMap::Append(value) => OutputMap::Append(f(value)),
        }
    }

    pub fn map_ref_with_err<U, E>(
        &self,
        f: impl FnOnce(&T) -> Result<U, E>,
    ) -> Result<OutputMap<U>, E> {
        match self {
            OutputMap::Print => Ok(OutputMap::Print),
            OutputMap::Create(value) => Ok(OutputMap::Create(f(value)?)),
            OutputMap::Append(value) => Ok(OutputMap::Append(f(value)?)),
        }
    }
}

#[derive(Clone, Debug)]
pub enum ArgBuilder {
    String(StringExpr),
    Set(VarFieldId),
}

impl ArgBuilder {
    pub fn evaluate<'a>(
        &'a self,
        state: &'a ProgramState,
    ) -> Result<ObjectIter<'a>, VariableAccessError> {
        match self {
            ArgBuilder::String(value) => Ok(ObjectIter::once(value.evaluate(state)?)),
            ArgBuilder::Set(value) => {
                let object = state.get_object(value)?;
                Ok(ObjectIter::object_iter(state, object))
            }
        }
    }
}

pub enum ObjectIter<'a> {
    Once(Option<String>),
    Iter { object: &'a Object, idx: usize },
}

impl<'a> ObjectIter<'a> {
    pub fn once(value: String) -> Self {
        Self::Once(Some(value))
    }

    pub fn object_iter(state: &'a ProgramState, object: &'a Object) -> Self {
        let object = match object {
            Object::Ref(variable_ref) => state.evaluate_ref(*variable_ref).unwrap(),
            object => object,
        };

        Self::Iter { object, idx: 0 }
    }
}

impl<'a> Iterator for ObjectIter<'a> {
    type Item = String;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            ObjectIter::Once(value) => value.take(),
            ObjectIter::Iter { object, idx } => match object {
                Object::Counter(counter) => {
                    if *idx >= counter.len() {
                        return None;
                    }

                    let value = counter.start + *idx as i64;
                    *idx += 1;
                    Some(format!("{value}"))
                }
                Object::Struct(value) => match *idx > 0 {
                    true => None,
                    false => {
                        *idx += 1;
                        Some(value.base.clone())
                    }
                },
                Object::List(vec) => {
                    let to_return = vec.get(*idx)?;
                    *idx += 1;
                    match to_return {
                        Object::Struct(value) => Some(value.base.clone()),
                        _ => panic!("Cannot iterate over list of non-struct values"),
                    }
                }
                Object::Ref(_) => unreachable!(),
            },
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
    pub fn evaluate(&self, state: &ProgramState) -> Result<ProcessInfo, VariableAccessError> {
        let command = self.command.evaluate(state)?;
        let mut process = ProcessInfo::new(command);

        for arg in self.args.iter() {
            let arg = arg.evaluate(state)?;
            process.args.extend(arg);
        }

        process
            .set_stdout(
                self.stdout
                    .map_ref_with_err(|value| Ok(value.evaluate(state)?.into()))?,
            )
            .set_stderr(
                self.stderr
                    .map_ref_with_err(|value| Ok(value.evaluate(state)?.into()))?,
            );

        if let Some(dir) = &self.working_dir {
            let working_dir = dir.evaluate(state)?;
            process.set_working_dir(working_dir.into());
        }

        Ok(process)
    }
}

#[derive(Clone, Debug)]
pub enum Command {
    LimitSpawn(usize),
    Sleep(u64),
    Spawn(Spawn),
    WaitAll(Option<u64>),
}
