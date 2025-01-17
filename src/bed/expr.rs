use std::collections::HashMap;

use crate::program::{
    Counter, IterTarget, Object, ProgramState, Struct, VarFieldId, VarNameId, VariableAccessError,
};

#[derive(Clone, Debug)]
pub enum StringInstance {
    String(String),
    Variable(VarFieldId),
}

#[derive(Clone, Debug, Default)]
pub struct StringExpr(pub Vec<StringInstance>);

impl StringExpr {
    pub fn evaluate(&self, state: &ProgramState) -> Result<String, VariableAccessError> {
        let mut output = String::new();

        for value in self.0.iter() {
            match value {
                StringInstance::String(value) => output.push_str(value),
                StringInstance::Variable(var) => {
                    let object = state.get_object(var)?;
                    object.write_to_string(state, &mut output)?;
                }
            }
        }

        Ok(output)
    }
}

#[derive(Clone, Debug)]
pub enum RangeExpr {
    Integer(i64),
    Variable(StringExpr),
}

impl RangeExpr {
    pub fn evaluate(&self, state: &ProgramState) -> Result<i64, VariableAccessError> {
        match self {
            RangeExpr::Integer(value) => Ok(*value),
            RangeExpr::Variable(value) => {
                let expr = value.evaluate(state)?;
                expr.parse().map_err(|_| VariableAccessError::InvalidIdx)
            }
        }
    }
}

#[derive(Clone, Debug)]
pub enum IterTargetExpr {
    Variable(VarNameId),
    Range { start: RangeExpr, end: RangeExpr },
}

impl IterTargetExpr {
    pub fn to_itertarget(&self) -> IterTarget {
        match self {
            IterTargetExpr::Variable(id) => IterTarget::Variable(*id),
            IterTargetExpr::Range { .. } => IterTarget::Range,
        }
    }
}

#[derive(Clone, Debug)]
pub struct StructExpr {
    pub base: StringExpr,
    pub properties: HashMap<VarNameId, ObjectExpr>,
}

#[derive(Clone, Debug)]
pub enum ObjectExpr {
    Clone(VarFieldId),
    List(Vec<ObjectExpr>),
    Counter(RangeExpr, RangeExpr),
    Struct(StructExpr),
}

impl ObjectExpr {
    pub fn evaluate(&self, state: &ProgramState) -> Result<Object, VariableAccessError> {
        match self {
            ObjectExpr::Clone(variable_ref) => {
                let object = state.get_object(variable_ref)?;
                Ok(object.clone())
            }
            ObjectExpr::List(list_expr) => {
                let mut list = Vec::with_capacity(list_expr.len());

                for value in list_expr {
                    list.push(value.evaluate(state)?);
                }

                Ok(Object::List(list))
            }
            ObjectExpr::Counter(min, max) => {
                let min = min.evaluate(state)?;
                let max = max.evaluate(state)?;

                Ok(Object::Counter(Counter {
                    offset: 0,
                    start: min,
                    end: max,
                }))
            }
            ObjectExpr::Struct(value) => {
                let mut properties = HashMap::default();

                for (key, value) in value.properties.iter() {
                    let object = value.evaluate(state)?;
                    properties.insert(*key, object);
                }

                Ok(Object::Struct(Struct::new(
                    value.base.evaluate(state)?,
                    properties,
                )))
            }
        }
    }
}
