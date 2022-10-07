use std::collections::HashMap;

use crate::program::{Object, ProgramState, VarFieldId, VarNameId, Variable};

#[derive(Clone, Debug)]
pub enum StringInstance {
    String(String),
    Variable(VarFieldId),
}

#[derive(Clone, Debug, Default)]
pub struct StringExpr(pub Vec<StringInstance>);

impl StringExpr {
    pub fn evaluate(&self, state: &ProgramState) -> String {
        let mut output = String::new();

        for value in self.0.iter() {
            match value {
                StringInstance::String(value) => output.push_str(value),
                StringInstance::Variable(var) => {
                    if let Some(value) = state.get_field(*var) {
                        output.push_str(value);
                    }
                }
            }
        }

        output
    }
}

#[derive(Clone, Debug, Default)]
pub struct ObjectExpr {
    pub base: StringExpr,
    pub properties: HashMap<VarNameId, StringExpr>,
}

impl ObjectExpr {
    pub fn new(base: StringExpr) -> Self {
        Self {
            base,
            properties: HashMap::new(),
        }
    }

    pub fn evaluate(&self, state: &ProgramState) -> Object {
        let base = self.base.evaluate(state);
        let mut object = Object::new(base);

        for (key, value) in self.properties.iter() {
            let value = value.evaluate(state);
            object.properties.insert(*key, value);
        }

        object
    }
}

#[derive(Clone, Debug)]
pub enum VariableExpr {
    Object(ObjectExpr),
    List(Vec<ObjectExpr>),
}

impl VariableExpr {
    pub fn evaluate(&self, state: &ProgramState) -> Variable {
        match self {
            VariableExpr::Object(object) => Variable::Object(object.evaluate(state)),
            VariableExpr::List(list) => {
                Variable::List(list.iter().map(|object| object.evaluate(state)).collect())
            }
        }
    }
}
