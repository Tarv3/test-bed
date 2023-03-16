use std::{
    collections::HashMap,
    fmt::Debug,
    ops::{Deref, DerefMut},
    sync::{atomic::AtomicBool, Arc},
};

use indexmap::IndexSet;

use crate::bed::expr::{ObjectExpr, VariableExpr};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct StackId(pub usize);

impl Deref for StackId {
    type Target = usize;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for StackId {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct InstructionId(pub usize);

impl Deref for InstructionId {
    type Target = usize;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for InstructionId {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[derive(Clone)]
pub struct Shutdown(Arc<AtomicBool>);

impl Shutdown {
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    pub fn shutdown(&self) -> bool {
        self.0.swap(true, std::sync::atomic::Ordering::Relaxed)
    }

    pub fn is_shutdown(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct VarNameId(pub usize);

impl Deref for VarNameId {
    type Target = usize;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for VarNameId {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[derive(Clone, Debug, Default)]
pub struct VarNames(pub IndexSet<String>);

impl VarNames {
    pub fn evaluate(&self, id: VarNameId) -> Option<&str> {
        self.0.get_index(*id).map(|value| value.as_str())
    }

    pub fn replace(&mut self, name: &str) -> VarNameId {
        let (id, _) = self.0.insert_full(name.into());

        VarNameId(id)
    }
}

#[derive(Clone, Debug)]
pub struct Object {
    pub base: String,
    pub properties: HashMap<VarNameId, String>,
}

impl Object {
    pub fn new(base: String) -> Self {
        Self {
            base,
            properties: HashMap::new(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct VariableRef {
    pub scope: usize,
    pub target: VarNameId,
    pub offset: usize,
}

#[derive(Clone, Debug)]
pub enum Variable {
    Ref(VariableRef),
    Object(Object),
    List(Vec<Object>),
}

impl Variable {
    pub fn is_ref(&self) -> bool {
        match self {
            Variable::Ref(_) => true,
            _ => false,
        }
    }

    pub fn len(&self) -> Option<usize> {
        match self {
            Variable::Ref(_) => None,
            Variable::Object(_) => Some(1),
            Variable::List(list) => Some(list.len()),
        }
    }

    pub fn is_list(&self) -> bool {
        match self {
            Variable::List(_) => true,
            _ => false,
        }
    }

    pub fn take_obj(self) -> Object {
        match self {
            Variable::Object(value) => value,
            _ => panic!("Tried to get non-object value as Object"),
        }
    }

    #[allow(dead_code)]
    pub fn as_obj(&mut self) -> &mut Object {
        match self {
            Variable::Object(value) => value,
            _ => panic!("Tried to get non-object value as Object"),
        }
    }

    pub fn as_list(&mut self) -> &mut Vec<Object> {
        match self {
            Variable::List(value) => value,
            _ => panic!("Tried to get non-list value as List"),
        }
    }

    pub fn as_ref(&mut self) -> &mut VariableRef {
        match self {
            Variable::Ref(value) => value,
            _ => panic!("Tried to get non-ref value as VariableRef"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Scope(pub HashMap<VarNameId, Variable>);

#[derive(Clone, Debug)]
pub enum VariableIdx {
    Integer(usize),
    Variable(VarFieldId),
}

#[derive(Clone, Debug)]
pub struct VarFieldId {
    pub var: VarNameId,
    pub idx: Option<Box<VariableIdx>>,
    pub field: Option<VarNameId>,
}

impl VarFieldId {
    pub fn new(var: VarNameId) -> Self {
        Self {
            var,
            idx: None,
            field: None,
        }
    }
}

#[derive(Clone, Copy)]
pub enum VarIter<I, T>
where
    I: Iterator<Item = T>,
{
    Single(T),
    List(I),
    None,
}

impl<I, T> Iterator for VarIter<I, T>
where
    I: Iterator<Item = T>,
{
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        let value = std::mem::replace(self, VarIter::None);

        match value {
            VarIter::Single(value) => {
                let to_return = value;
                Some(to_return)
            }
            VarIter::List(mut value) => {
                let next = value.next();
                if next.is_some() {
                    *self = VarIter::List(value);
                }
                next
            }
            VarIter::None => None,
        }
    }
}

pub struct ProgramState {
    pub scopes: Vec<Scope>,

    scope_cache: Vec<Scope>,
    list_cache: Vec<Vec<Object>>,
    obj_cache: Vec<Object>,
}

impl ProgramState {
    pub fn new() -> Self {
        Self {
            scopes: vec![],
            scope_cache: vec![],
            list_cache: vec![],
            obj_cache: vec![],
        }
    }

    pub fn new_scope(&mut self) {
        let scope = self.scope_cache.pop().unwrap_or(Scope(HashMap::new()));
        self.scopes.push(scope);
    }

    pub fn evaluate_ref(&self, value: VariableRef) -> Option<&Object> {
        let scope = &self.scopes[value.scope];
        let mut variable = scope.0.get(&value.target)?;

        while let Variable::Ref(value) = variable {
            let scope = &self.scopes[value.scope];
            variable = scope.0.get(&value.target)?;

            if let Variable::List(list) = variable {
                return list.get(value.offset);
            }
        }

        match variable {
            Variable::Object(value) => Some(value),
            Variable::List(list) => list.first(),
            _ => unreachable!(),
        }
    }

    pub fn get_object(&self, id: VarNameId, idx: Option<usize>) -> Option<&Object> {
        let (_, mut variable) = self.get_value(id)?;

        while let Variable::Ref(value) = variable {
            if idx.is_some() {
                panic!("Tried to index into a reference");
            }

            let scope = &self.scopes[value.scope];
            variable = scope.0.get(&value.target)?;

            if let Variable::List(list) = variable {
                return list.get(value.offset);
            }
        }

        match variable {
            Variable::Object(value) => {
                if idx.is_some() {
                    panic!("Tried to index into an object");
                }
                Some(value)
            }
            Variable::List(list) => {
                let idx = idx.unwrap_or(0);
                list.get(idx)
            }
            _ => unreachable!(),
        }
    }

    pub fn get_field(&self, id: &VarFieldId) -> Option<&str> {
        let idx = match &id.idx {
            Some(value) => match value.as_ref() {
                VariableIdx::Integer(idx) => Some(*idx),
                VariableIdx::Variable(id) => {
                    let value = self.get_field(id)?;
                    match value.parse() {
                        Ok(idx) => Some(idx),
                        Err(_) => panic!("List cannot be indexed by {value}",),
                    }
                }
            },
            None => None,
        };

        let object = self.get_object(id.var, idx)?;

        match id.field {
            Some(field) => object.properties.get(&field).map(|value| value.as_str()),
            None => Some(&object.base),
        }
    }

    pub fn get_var<'a>(&'a self, id: VarNameId) -> impl Iterator<Item = &'a Object> {
        let (_, mut variable) = match self.get_value(id) {
            Some(value) => value,
            None => return VarIter::None,
        };

        while let Variable::Ref(value) = variable {
            let scope = &self.scopes[value.scope];
            variable = match scope.0.get(&value.target) {
                Some(value) => value,
                None => return VarIter::None,
            };

            match variable {
                Variable::Object(object) => return VarIter::Single(object),
                Variable::List(list) => return VarIter::List(list.iter()),
                Variable::Ref(_) => {}
            }
        }

        match variable {
            Variable::Object(object) => VarIter::Single(object),
            Variable::List(list) => VarIter::List(list.iter()),
            Variable::Ref(_) => unreachable!(),
        }
    }

    pub fn pop_scope(&mut self) {
        let mut scope = match self.scopes.pop() {
            Some(scope) => scope,
            None => return,
        };

        for (_, variable) in scope.0.drain() {
            match variable {
                Variable::Object(obj) => self.obj_cache.push(obj),
                Variable::List(mut objs) => {
                    self.obj_cache.extend(objs.drain(..));
                    self.list_cache.push(objs);
                }
                _ => {}
            }
        }

        self.scope_cache.push(scope);
    }

    pub fn insert_var(
        &mut self,
        id: VarNameId,
        var: Variable,
        scope: Option<usize>,
    ) -> &mut Variable {
        if self.scopes.is_empty() {
            self.new_scope();
        }
        let scope = scope.unwrap_or(self.scopes.len() - 1);

        while scope >= self.scopes.len() {
            self.new_scope();
        }

        let scope = &mut self.scopes[scope];
        scope.0.insert(id, var);
        scope.0.get_mut(&id).unwrap()
    }

    pub fn new_list(&mut self, id: VarNameId, scope: Option<usize>) -> &mut Vec<Object> {
        let list = self.list_cache.pop().unwrap_or(vec![]);
        let var = Variable::List(list);

        self.insert_var(id, var, scope).as_list()
    }

    #[allow(dead_code)]
    pub fn new_obj(&mut self, id: VarNameId, scope: Option<usize>) -> &mut Object {
        let obj = self.obj_cache.pop().unwrap_or(Object {
            base: String::new(),
            properties: HashMap::new(),
        });
        let var = Variable::Object(obj);

        self.insert_var(id, var, scope).as_obj()
    }

    pub fn new_ref(
        &mut self,
        id: VarNameId,
        target: VarNameId,
        offset: usize,
        target_scope: usize,
        scope: Option<usize>,
    ) -> &mut VariableRef {
        let var = Variable::Ref(VariableRef {
            scope: target_scope,
            target,
            offset,
        });

        self.insert_var(id, var, scope).as_ref()
    }

    pub fn get_value(&self, variable: VarNameId) -> Option<(usize, &Variable)> {
        for (i, scope) in self.scopes.iter().enumerate().rev() {
            if let Some(value) = scope.0.get(&variable) {
                return Some((i, value));
            }
        }

        None
    }

    pub fn get_value_mut(&mut self, variable: VarNameId) -> Option<&mut Variable> {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(value) = scope.0.get_mut(&variable) {
                return Some(value);
            }
        }

        None
    }
}

pub trait Executable<Command> {
    fn shutdown(&mut self);

    fn finish(&mut self, state: &mut ProgramState, shutdown: &Shutdown);

    fn execute(&mut self, command: &Command, state: &mut ProgramState, shutdown: &Shutdown);
}

#[derive(Clone, Debug)]
pub enum Instruction<T> {
    PushScope,
    PopScope,
    PushList {
        target: VarNameId,
        object: ObjectExpr,
    },
    AssignVar {
        target: VarNameId,
        scope: Option<usize>,
        value: VariableExpr,
    },
    StartIter {
        target: VarNameId,
        iter: VarNameId,
        end: InstructionId,
    },
    Increment {
        target: VarNameId,
        iter: VarNameId,
        end: InstructionId,
    },
    Goto(InstructionId),
    Command(T),
}

#[derive(Clone, Debug)]
pub struct Program<T>(pub Vec<Instruction<T>>);

impl<T: Debug> std::fmt::Display for Program<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, value) in self.0.iter().enumerate() {
            writeln!(f, "{i}: {value:?}")?
        }

        Ok(())
    }
}

impl<Command> Program<Command> {
    pub fn run(
        &self,
        executable: &mut impl Executable<Command>,
        state: &mut ProgramState,
        shutdown: &Shutdown,
    ) {
        let mut counter = 0;

        while counter < self.0.len() {
            if shutdown.is_shutdown() {
                executable.shutdown();
                return;
            }

            let instruction = &self.0[counter];

            match instruction {
                Instruction::PushScope => {
                    state.new_scope();
                }
                Instruction::PopScope => {
                    state.pop_scope();
                }
                Instruction::PushList { target, object } => {
                    let object = object.evaluate(state);
                    match state.get_value_mut(*target) {
                        Some(Variable::Ref(_)) => panic!("Tried to push to reference variable"),
                        Some(variable) => {
                            if !variable.is_list() {
                                let mut new_var = Variable::List(vec![]);
                                std::mem::swap(variable, &mut new_var);
                                let object = new_var.take_obj();
                                variable.as_list().push(object);
                            }

                            variable.as_list().push(object);
                        }
                        None => {
                            state.insert_var(*target, Variable::Object(object), None);
                        }
                    }
                }
                Instruction::AssignVar {
                    target,
                    scope,
                    value,
                } => {
                    let eval = value.evaluate(state);
                    match scope {
                        Some(scope) => {
                            if let Some(scope) = state.scopes.get_mut(*scope) {
                                scope.0.insert(*target, eval);
                            }
                        }
                        None => {
                            state.insert_var(*target, eval, None);
                        }
                    }
                }
                Instruction::StartIter { target, iter, end } => match state.get_value(*target) {
                    Some((scope, value)) if value.len().is_some() && value.len().unwrap() > 0 => {
                        state.new_ref(*iter, *target, 0, scope, None);
                    }
                    _ => {
                        counter = **end;
                        continue;
                    }
                },
                Instruction::Increment { target, iter, end } => {
                    let len = match state.get_value(*target).map(|(_, value)| value.len()) {
                        Some(Some(len)) => len,
                        _ => {
                            counter = **end;
                            continue;
                        }
                    };

                    let iter = match state.get_value_mut(*iter) {
                        Some(value) if value.is_ref() => value.as_ref(),
                        _ => {
                            counter = **end;
                            continue;
                        }
                    };

                    iter.offset += 1;

                    if iter.offset >= len {
                        counter = **end;
                        continue;
                    }
                }
                Instruction::Goto(target) => {
                    counter = **target;
                    continue;
                }
                Instruction::Command(command) => {
                    executable.execute(command, state, shutdown);
                }
            }

            counter += 1;
        }

        executable.finish(state, shutdown);
    }
}
