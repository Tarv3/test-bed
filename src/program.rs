use std::{
    collections::HashMap,
    fmt::Debug,
    ops::{Deref, DerefMut},
    sync::{atomic::AtomicBool, Arc},
};

use indexmap::IndexSet;
use serde::{
    ser::{SerializeMap, SerializeSeq},
    Serialize,
};

use crate::bed::expr::{IterTargetExpr, ObjectExpr};

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
pub struct Struct {
    pub base: String,
    pub properties: HashMap<VarNameId, Object>,
}

impl Struct {
    pub fn new(base: String, properties: HashMap<VarNameId, Object>) -> Self {
        Self { base, properties }
    }
}

#[derive(Clone, Debug)]
pub enum Object {
    Counter(Counter),
    Ref(VariableRef),
    Struct(Struct),
    List(Vec<Object>),
}

impl Object {
    pub fn new(base: String) -> Self {
        Self::Struct(Struct {
            base,
            properties: HashMap::new(),
        })
    }

    pub fn write_to_string<'a>(
        &'a self,
        state: &'a ProgramState,
        mut into: impl std::fmt::Write,
    ) -> Result<(), VariableAccessError> {
        match self {
            Object::Struct(value) => {
                write!(into, "{}", &value.base).unwrap();
            }
            Object::Ref(variable_ref) => state
                .evaluate_ref(*variable_ref)
                .unwrap()
                .write_to_string(state, into)?,
            Object::Counter(counter) => {
                write!(into, "{}", counter.idx()).unwrap();
            }
            Object::List(_) => return Err(VariableAccessError::NotAStruct),
        }

        Ok(())
    }

    pub fn to_serialize<'a>(
        &'a self,
        program: &'a ProgramState,
        names: &'a VarNames,
    ) -> ObjectSerialize<'a> {
        ObjectSerialize {
            object: &self,
            // base: &self.base,
            // properties: &self.properties,
            program,
            names,
        }
    }
}

pub struct ObjectSerialize<'a> {
    object: &'a Object,
    // base: &'a str,
    // properties: &'a HashMap<VarNameId, Object>,
    program: &'a ProgramState,
    names: &'a VarNames,
}

impl<'a> Serialize for ObjectSerialize<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self.object {
            Object::Counter(counter) => return serializer.serialize_i64(counter.idx()),
            Object::Ref(variable_ref) => {
                let Some(object) = self.program.evaluate_ref(*variable_ref) else {
                    return Err(serde::ser::Error::custom(
                        "Failed to evaluate variable reference",
                    ));
                };

                return object
                    .to_serialize(&self.program, &self.names)
                    .serialize(serializer);
            }
            Object::Struct(value) => {
                if value.properties.is_empty() {
                    return value.base.serialize(serializer);
                }

                let mut map_serialize = serializer.serialize_map(Some(2))?;
                map_serialize.serialize_entry(&"base", &value.base)?;
                map_serialize.serialize_entry(
                    &"properties",
                    &PropertiesSerialize {
                        properties: &value.properties,
                        program: self.program,
                        names: self.names,
                    },
                )?;

                return map_serialize.end();
            }
            Object::List(vec) => {
                let mut seq_serialize = serializer.serialize_seq(Some(vec.len()))?;
                for value in vec.iter() {
                    seq_serialize
                        .serialize_element(&value.to_serialize(&self.program, &self.names))?;
                }
                return seq_serialize.end();
            }
        }
    }
}

struct PropertiesSerialize<'a> {
    properties: &'a HashMap<VarNameId, Object>,
    program: &'a ProgramState,
    names: &'a VarNames,
}

impl<'a> Serialize for PropertiesSerialize<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut map_serialize = serializer.serialize_map(Some(self.properties.len()))?;

        for (key, value) in self.properties.iter() {
            let Some(name) = self.names.evaluate(*key) else {
                return Err(serde::ser::Error::custom("Missing name for key"));
            };
            let serialize = value.to_serialize(self.program, self.names);
            map_serialize.serialize_entry(name, &serialize)?;
        }

        map_serialize.end()
    }
}

// pub enum VariableDeref<'a> {
//     Object(&'a Object),
//     Counter(&'a Counter),
// }

// pub enum VariableField<'a> {
//     // String(&'a str),
//     Object(&'a Object),
//     Idx(i64),
// }

#[derive(Clone, Copy, Debug)]
pub struct VariableRef {
    pub scope: usize,
    pub target: VarNameId,
    pub offset: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct Counter {
    pub offset: usize,
    pub start: i64,
    pub end: i64,
}

impl Counter {
    pub fn idx(&self) -> i64 {
        self.start + self.offset as i64
    }

    pub fn len(&self) -> usize {
        let value = match self.end >= self.start {
            true => self.end - self.start,
            false => 0,
        };

        value as usize
    }
}

// #[derive(Clone, Debug)]
// pub enum Variable {
//     Counter(Counter),
//     Ref(VariableRef),
//     Object(Object),
//     List(Vec<Object>),
// }

// impl Variable {
//     pub fn is_ref(&self) -> bool {
//         match self {
//             Variable::Ref(_) => true,
//             _ => false,
//         }
//     }

//     pub fn is_counter(&self) -> bool {
//         match self {
//             Variable::Counter(_) => true,
//             _ => false,
//         }
//     }

//     pub fn len(&self) -> Option<usize> {
//         match self {
//             Variable::Counter(value) => Some(value.len()),
//             Variable::Ref(_) => None,
//             Variable::Object(_) => Some(1),
//             Variable::List(list) => Some(list.len()),
//         }
//     }

//     pub fn is_list(&self) -> bool {
//         match self {
//             Variable::List(_) => true,
//             _ => false,
//         }
//     }

//     pub fn take_obj(self) -> Object {
//         match self {
//             Variable::Object(value) => value,
//             _ => panic!("Tried to get non-object value as Object"),
//         }
//     }

//     #[allow(dead_code)]
//     pub fn as_obj(&mut self) -> &mut Object {
//         match self {
//             Variable::Object(value) => value,
//             _ => panic!("Tried to get non-object value as Object"),
//         }
//     }

//     pub fn as_list(&mut self) -> &mut Vec<Object> {
//         match self {
//             Variable::List(value) => value,
//             _ => panic!("Tried to get non-list value as List"),
//         }
//     }

//     pub fn as_ref(&mut self) -> &mut VariableRef {
//         match self {
//             Variable::Ref(value) => value,
//             _ => panic!("Tried to get non-ref value as VariableRef"),
//         }
//     }

//     pub fn as_counter(&mut self) -> &mut Counter {
//         match self {
//             Variable::Counter(value) => value,
//             _ => panic!("Tried to get non-ref value as VariableRef"),
//         }
//     }

//     pub fn to_serialize<'a>(
//         &'a self,
//         state: &'a ProgramState,
//         names: &'a VarNames,
//     ) -> VariableSerialize<'a> {
//         VariableSerialize {
//             variable: self,
//             state,
//             names,
//         }
//     }
// }

// pub struct VariableSerialize<'a> {
//     variable: &'a Variable,
//     state: &'a ProgramState,
//     names: &'a VarNames,
// }

// impl<'a> Serialize for VariableSerialize<'a> {
//     fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
//     where
//         S: serde::Serializer,
//     {
//         match self.variable {
//             Variable::Counter(counter) => serializer.serialize_i64(counter.idx()),
//             Variable::Ref(variable_ref) => {
//                 let Some(object) = self.state.evaluate_ref(*variable_ref) else {
//                     return Err(serde::ser::Error::custom("Missing referenced object"));
//                 };

//                 object.to_serialize(self.names).serialize(serializer)
//             }
//             Variable::Object(object) => object.to_serialize(self.names).serialize(serializer),
//             Variable::List(vec) => {
//                 let mut seq_serializer = serializer.serialize_seq(Some(vec.len()))?;
//                 for value in vec.iter() {
//                     seq_serializer.serialize_element(&value.to_serialize(self.names))?;
//                 }

//                 seq_serializer.end()
//             }
//         }
//     }
// }

#[derive(Clone, Debug)]
pub struct Scope(pub HashMap<VarNameId, Object>);

#[derive(Clone, Debug)]
pub enum VariableIdx {
    Integer(usize),
    Variable(VarFieldId),
}

#[derive(Clone, Debug)]
pub struct VarFieldId {
    pub var: VarNameId,
    pub idx: Option<Box<VariableIdx>>,
    pub field: Option<Box<VarFieldId>>,
}

impl VarFieldId {
    pub fn new(var: VarNameId) -> Self {
        Self {
            var,
            idx: None,
            field: None,
        }
    }

    pub fn get_value<'a>(
        &self,
        program: &'a ProgramState,
        object: &'a Object,
    ) -> Result<&'a Object, VariableAccessError> {
        let properties = match object {
            Object::Struct(value) => &value.properties,
            Object::Ref(value) => match program.evaluate_ref(*value).unwrap() {
                Object::Struct(value) => &value.properties,
                _x => {
                    return Err(VariableAccessError::NotAStruct);
                }
            },
            _x => {
                return Err(VariableAccessError::NotAStruct);
            }
        };

        let Some(mut output) = properties.get(&self.var) else {
            return Err(VariableAccessError::MissingField(self.var));
        };

        if let Some(idx) = &self.idx {
            let Object::List(list) = output else {
                return Err(VariableAccessError::NotAList);
            };

            let idx = program.evaluate_idx(idx)?;
            output = idx.get_object(list)?;
        }

        if let Some(field) = &self.field {
            output = field.get_value(program, output)?;
        }

        Ok(output)
    }
}

#[derive(Clone, Debug)]
pub enum VariableAccessError {
    NotAStruct,
    NotARef,
    NotAList,
    InvalidIdx,
    MissingVariable(VarNameId),
    MissingField(VarNameId),
}

impl std::fmt::Display for VariableAccessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

#[derive(Clone, Copy)]
pub enum ListIdx<'a> {
    Integer(usize),
    String(&'a str),
}

impl<'a> ListIdx<'a> {
    pub fn get_object<'b>(&self, list: &'b [Object]) -> Result<&'b Object, VariableAccessError> {
        match self {
            ListIdx::Integer(idx) => match list.get(*idx) {
                Some(object) => Ok(object),
                None => Err(VariableAccessError::InvalidIdx),
            },
            ListIdx::String(str) => {
                for value in list.iter() {
                    let base = match value {
                        Object::Struct(value) => &value.base,
                        _ => continue,
                    };

                    if base == str {
                        return Ok(value);
                    }
                }

                Err(VariableAccessError::InvalidIdx)
            }
        }
    }
}

pub struct ProgramState {
    pub scopes: Vec<Scope>,

    scope_cache: Vec<Scope>,
}

impl ProgramState {
    pub fn new() -> Self {
        Self {
            scopes: vec![],
            scope_cache: vec![],
        }
    }

    pub fn new_scope(&mut self) {
        let scope = self.scope_cache.pop().unwrap_or(Scope(HashMap::new()));
        self.scopes.push(scope);
    }

    pub fn evaluate_ref(&self, value: VariableRef) -> Option<&Object> {
        let scope = &self.scopes[value.scope];
        let mut variable = scope.0.get(&value.target)?;

        if let Object::List(list) = variable {
            variable = list.get(value.offset)?;
        }

        while let Object::Ref(value) = variable {
            let scope = &self.scopes[value.scope];
            variable = scope.0.get(&value.target)?;

            if let Object::List(list) = variable {
                variable = list.get(value.offset)?;
            }
        }

        Some(variable)
    }

    // pub fn get_object(&self, id: VarNameId, idx: Option<ListIdx>) -> Option<VariableDeref> {
    //     let (_, mut variable) = self.get_value(id)?;

    //     if let Variable::Counter(counter) = variable {
    //         return Some(VariableDeref::Counter(counter));
    //     }

    //     while let Variable::Ref(value) = variable {
    //         if idx.is_some() {
    //             panic!("Tried to index into a reference");
    //         }

    //         let scope = &self.scopes[value.scope];
    //         variable = scope.0.get(&value.target)?;

    //         if let Variable::List(list) = variable {
    //             return Some(VariableDeref::Object(list.get(value.offset)?));
    //         }
    //     }

    //     match variable {
    //         Variable::Object(value) => {
    //             if idx.is_some() {
    //                 panic!("Tried to index into an object");
    //             }
    //             Some(VariableDeref::Object(value))
    //         }
    //         Variable::List(list) => {
    //             let idx = idx.unwrap_or(ListIdx::Integer(0));

    //             match idx {
    //                 ListIdx::Integer(idx) => Some(VariableDeref::Object(list.get(idx)?)),
    //                 ListIdx::String(lookup) => {
    //                     let value = list.iter().find(|value| value.base == lookup)?;
    //                     Some(VariableDeref::Object(value))
    //                 }
    //             }
    //         }
    //         _ => unreachable!(),
    //     }
    // }

    pub fn object_to_idx<'a>(&'a self, object: &'a Object) -> Option<ListIdx<'a>> {
        match object {
            Object::Counter(counter) => {
                let idx = counter.idx();

                if idx < 0 {
                    return None;
                };

                Some(ListIdx::Integer(idx as usize))
            }
            Object::Ref(variable_ref) => {
                let Some(object) = self.evaluate_ref(*variable_ref) else {
                    return None;
                };

                self.object_to_idx(object)
            }
            Object::Struct(value) => match value.base.parse() {
                Ok(idx) => Some(ListIdx::Integer(idx)),
                Err(_) => Some(ListIdx::String(&value.base)),
            },
            Object::List(_) => None,
        }
    }

    pub fn evaluate_idx<'a>(
        &'a self,
        idx: &VariableIdx,
    ) -> Result<ListIdx<'a>, VariableAccessError> {
        let id = match idx {
            VariableIdx::Integer(idx) => return Ok(ListIdx::Integer(*idx)),
            VariableIdx::Variable(id) => id,
        };

        let object = self.get_object(id)?;
        let Some(idx) = self.object_to_idx(object) else {
            return Err(VariableAccessError::InvalidIdx);
        };

        Ok(idx)
    }

    pub fn get_object<'a>(&'a self, id: &VarFieldId) -> Result<&'a Object, VariableAccessError> {
        let Some((_scope_idx, mut object)) = self.get_value(id.var) else {
            return Err(VariableAccessError::MissingVariable(id.var));
        };

        if let Some(idx) = &id.idx {
            let Object::List(list) = object else {
                return Err(VariableAccessError::NotAList);
            };

            let idx = self.evaluate_idx(idx)?;
            object = idx.get_object(list)?;
        }

        if let Some(field) = &id.field {
            object = field.get_value(self, object)?;
        }

        Ok(object)
    }

    pub fn pop_scope(&mut self) {
        let mut scope = match self.scopes.pop() {
            Some(scope) => scope,
            None => return,
        };

        scope.0.clear();
        self.scope_cache.push(scope);
    }

    pub fn insert_var(&mut self, id: VarNameId, var: Object, scope: Option<usize>) -> &mut Object {
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

    pub fn set_var(
        &mut self,
        id: VarNameId,
        property: Option<VarNameId>,
        value: Object,
    ) -> Result<(), VariableAccessError> {
        match property {
            Some(property_id) => match self.get_value_mut(id) {
                Some(Object::Struct(into)) => {
                    into.properties.insert(property_id, value);
                }
                Some(_) => return Err(VariableAccessError::NotAStruct),
                None => return Err(VariableAccessError::MissingVariable(id)),
            },
            None => match self.get_value_mut(id) {
                Some(variable) => {
                    *variable = value;
                }
                None => {
                    if self.scopes.is_empty() {
                        self.new_scope();
                    }
                    let scope = self.scopes.last_mut().unwrap();
                    scope.0.insert(id, value);
                }
            },
        }

        Ok(())
    }

    pub fn get_value(&self, variable: VarNameId) -> Option<(usize, &Object)> {
        for (i, scope) in self.scopes.iter().enumerate().rev() {
            if let Some(value) = scope.0.get(&variable) {
                return Some((i, value));
            }
        }

        None
    }

    pub fn get_value_mut(&mut self, variable: VarNameId) -> Option<&mut Object> {
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

    fn execute(
        &mut self,
        command: &Command,
        state: &mut ProgramState,
        shutdown: &Shutdown,
    ) -> Result<(), VariableAccessError>;

    fn set_iter(&mut self, iter_var: VarNameId, idx: usize, variable: &Object) {
        let _iter_var = iter_var;
        let _idx = idx;
        let _variable = variable;
    }
}

#[derive(Clone, Debug)]
pub enum IterTarget {
    Variable(VarNameId),
    Range,
}

#[derive(Clone, Debug)]
pub enum Instruction<T> {
    PushScope,
    PopScope,
    PushList {
        target: VarNameId,
        object: ObjectExpr,
    },
    CreateVar {
        target: VarNameId,
        scope: Option<usize>,
        value: ObjectExpr,
    },
    AssignVar {
        target: VarNameId,
        scope: Option<usize>,
        value: ObjectExpr,
    },
    StartIter {
        /// Id of the variable to iterate over
        target: IterTargetExpr,
        /// Id of the variable used inside the iter
        iter: VarNameId,
        jump: InstructionId,
    },
    Increment {
        target: IterTarget,
        iter: VarNameId,
        jump: InstructionId,
    },
    ConditionalJump {
        cond: VarFieldId,
        jump: InstructionId,
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
    ) -> Result<(), (usize, VariableAccessError)> {
        let mut counter = 0;

        while counter < self.0.len() {
            if shutdown.is_shutdown() {
                executable.shutdown();
                return Ok(());
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
                    let object = object.evaluate(state).map_err(|e| (counter, e))?;

                    match state.get_value_mut(*target) {
                        Some(Object::List(list)) => {
                            list.push(object);
                        }
                        Some(_) => return Err((counter, VariableAccessError::NotAList)),
                        None => {
                            return Err((counter, VariableAccessError::MissingVariable(*target)))
                        }
                    }
                }
                Instruction::CreateVar {
                    target,
                    scope,
                    value,
                } => {
                    let eval = value.evaluate(state).map_err(|e| (counter, e))?;
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
                Instruction::AssignVar {
                    target,
                    scope,
                    value,
                } => {
                    let eval = value.evaluate(state).map_err(|e| (counter, e))?;
                    match scope {
                        Some(scope) => {
                            if let Some(scope) = state.scopes.get_mut(*scope) {
                                if let Some(variable) = scope.0.get_mut(target) {
                                    *variable = eval;
                                }
                            }
                        }
                        None => {
                            if let Some(variable) = state.get_value_mut(*target) {
                                *variable = eval;
                            }
                        }
                    }
                }
                Instruction::StartIter {
                    target: IterTargetExpr::Variable(target),
                    iter,
                    jump,
                } => {
                    let (scope, object) = state
                        .get_value(*target)
                        .ok_or((counter, VariableAccessError::MissingVariable(*target)))?;

                    let len = match object {
                        Object::List(vec) => vec.len(),
                        Object::Counter(counter) => counter.len(),
                        _ => return Err((counter, VariableAccessError::NotAList)),
                    };

                    if len > 0 {
                        executable.set_iter(*iter, 0, object);
                        state.insert_var(
                            *iter,
                            Object::Ref(VariableRef {
                                scope,
                                target: *target,
                                offset: 0,
                            }),
                            None,
                        );
                    } else {
                        counter = **jump;
                        continue;
                    };
                }
                Instruction::Increment {
                    target: IterTarget::Variable(target),
                    iter,
                    jump,
                } => {
                    let (_scope, object) = state
                        .get_value(*target)
                        .ok_or((counter, VariableAccessError::MissingVariable(*target)))?;

                    let len = match object {
                        Object::List(vec) => vec.len(),
                        Object::Counter(counter) => counter.len(),
                        _ => return Err((counter, VariableAccessError::NotAList)),
                    };

                    let iter_var = state
                        .get_value_mut(*iter)
                        .ok_or((counter, VariableAccessError::MissingVariable(*iter)))?;

                    let Object::Ref(iter_var) = iter_var else {
                        return Err((counter, VariableAccessError::NotARef));
                    };

                    iter_var.offset += 1;
                    let offset = iter_var.offset;
                    let variable = state.get_value(*target).unwrap().1;
                    executable.set_iter(*iter, offset, variable);

                    if offset >= len {
                        counter = **jump;
                        continue;
                    }
                }
                Instruction::StartIter {
                    target: IterTargetExpr::Range { start, end },
                    iter,
                    jump,
                } => {
                    let start = start.evaluate(state).map_err(|e| (counter, e))?;
                    let end = end.evaluate(state).map_err(|e| (counter, e))?;

                    if start >= end {
                        counter = **jump;
                        continue;
                    }

                    let var = Object::Counter(Counter {
                        offset: 0,
                        start,
                        end,
                    });
                    let var = state.insert_var(*iter, var, None);
                    executable.set_iter(*iter, 0, var);
                }
                Instruction::Increment {
                    target: IterTarget::Range,
                    iter,
                    jump,
                } => {
                    let iter_var = state
                        .get_value_mut(*iter)
                        .ok_or((counter, VariableAccessError::MissingVariable(*iter)))?;

                    let Object::Counter(range_counter) = iter_var else {
                        return Err((counter, VariableAccessError::NotARef));
                    };

                    range_counter.offset += 1;
                    let idx = range_counter.start + range_counter.offset as i64;
                    let end = range_counter.end;
                    let offset = range_counter.offset;
                    executable.set_iter(*iter, offset, iter_var);

                    if idx >= end {
                        counter = **jump;
                        continue;
                    }
                }
                Instruction::ConditionalJump { cond, jump } => {
                    let object = state.get_object(cond).map_err(|e| (counter, e))?;

                    let value = match object {
                        Object::Struct(object) => object,
                        Object::Ref(variable_ref) => {
                            match state.evaluate_ref(*variable_ref).unwrap() {
                                Object::Struct(object) => object,
                                _ => return Err((counter, VariableAccessError::NotAStruct)),
                            }
                        }
                        _ => return Err((counter, VariableAccessError::NotAStruct)),
                    };

                    if value.base != "false" {
                        counter = **jump;
                        continue;
                    }
                }
                Instruction::Goto(target) => {
                    counter = **target;
                    continue;
                }
                Instruction::Command(command) => {
                    if let Err(e) = executable.execute(command, state, shutdown) {
                        return Err((counter, e));
                    }
                }
            }

            counter += 1;
        }

        executable.finish(state, shutdown);
        Ok(())
    }
}
