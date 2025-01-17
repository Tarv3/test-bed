use std::{collections::HashMap, fmt::Display, io::ErrorKind, path::PathBuf};

use minijinja::{Environment, Source};

use crate::program::{
    Object, ObjectSerialize, ProgramState, Struct, VarNameId, VarNames, VariableAccessError,
};

use super::expr::{ObjectExpr, StringExpr};

#[derive(Debug)]
pub enum TemplateErrorType {
    InvalidPath(PathBuf),
    WriteError(std::io::Error),
    RenderError(minijinja::Error),
}

pub enum TemplateBuildError {
    VariableError(VariableAccessError),
    BuildError {
        template_path: String,
        output_path: String,
        error: TemplateErrorType,
    },
}

impl From<VariableAccessError> for TemplateBuildError {
    fn from(value: VariableAccessError) -> Self {
        Self::VariableError(value)
    }
}

impl Display for TemplateBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TemplateBuildError::VariableError(variable_access_error) => {
                write!(f, "{variable_access_error}")
            }
            TemplateBuildError::BuildError {
                template_path,
                output_path,
                error,
            } => {
                write!(f, "Template `{}` -> `{}` ", template_path, output_path)?;

                match &error {
                    TemplateErrorType::InvalidPath(path) => {
                        write!(f, "path `{path:?}` could not be converted to string")
                    }
                    TemplateErrorType::WriteError(e) => {
                        write!(f, "failed to save render: {e}")
                    }
                    TemplateErrorType::RenderError(e) => {
                        write!(f, "failed to render template: {e}")
                    }
                }
            }
        }
    }
}

pub struct TemplateBuilder<'source> {
    pub environment: Environment<'source>,
    output: PathBuf,
}

impl<'source> TemplateBuilder<'source> {
    pub fn new(output: PathBuf, paths: Vec<PathBuf>) -> Self {
        let mut env = Environment::new();
        let source = Source::with_loader(move |path| {
            for parent in paths.iter() {
                let mut child = parent.clone();
                child.push(path);

                if let Ok(value) = std::fs::read_to_string(child) {
                    return Ok(Some(value));
                }
            }

            Ok(None)
        });

        std::fs::create_dir_all(&output).expect("Failed to create output dir");

        env.set_source(source);

        Self {
            environment: env,
            output,
        }
    }

    pub fn build(
        &mut self,
        template_path: String,
        output_name: String,
        state: &ProgramState,
        names: &VarNames,
    ) -> Result<String, TemplateBuildError> {
        let template = match self.environment.get_template(&template_path) {
            Ok(template) => template,
            Err(e) => {
                todo!("Template Error: {e}")
            }
        };

        let mut current_params: HashMap<&str, ObjectSerialize> = Default::default();
        // self.current_params.clear();

        for scope in state.scopes.iter().rev() {
            for (name, value) in scope.0.iter() {
                let name = match names.evaluate(*name) {
                    Some(name) => name,
                    None => continue,
                };

                if current_params.contains_key(name) {
                    continue;
                }

                let value = value.to_serialize(state, names);
                current_params.insert(name, value);
            }
        }

        let mut output_file = self.output.clone();
        output_file.push(output_name);

        let output_path = match output_file.to_str() {
            Some(file) => file.to_string(),
            None => {
                return Err(TemplateBuildError::BuildError {
                    template_path,
                    output_path: output_file.to_string_lossy().to_string(),
                    error: TemplateErrorType::InvalidPath(output_file),
                })
            }
        };

        let rendered = match template.render(&current_params) {
            Ok(rendered) => rendered,
            Err(e) => {
                return Err(TemplateBuildError::BuildError {
                    template_path,
                    output_path,
                    error: TemplateErrorType::RenderError(e),
                })
            }
        };

        if let Some(parent) = output_file.parent() {
            match std::fs::create_dir_all(parent) {
                Ok(_) => {}
                Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
                Err(e) => {
                    return Err(TemplateBuildError::BuildError {
                        template_path,
                        output_path,
                        error: TemplateErrorType::WriteError(e),
                    })
                }
            }
        }

        if let Err(e) = std::fs::write(&output_file, rendered) {
            return Err(TemplateBuildError::BuildError {
                template_path,
                output_path,
                error: TemplateErrorType::WriteError(e),
            });
        };

        Ok(output_path)
    }
}

pub fn yield_value(output: VarNameId, to_yield: Object, state: &mut ProgramState) {
    match state.scopes[0].0.get_mut(&output) {
        Some(Object::List(list)) => {
            list.push(to_yield);
        }
        _ => {
            state.scopes[0]
                .0
                .insert(output, Object::List(vec![to_yield]));
        }
    };
}

#[derive(Clone, Debug)]
pub struct BuildStringExpr {
    pub template: StringExpr,
    pub output: StringExpr,
}

impl BuildStringExpr {
    pub fn evaluate<'a>(
        &self,
        state: &mut ProgramState,
        builder: &mut TemplateBuilder<'a>,
        names: &VarNames,
    ) -> Result<String, TemplateBuildError> {
        let template = self.template.evaluate(state)?;
        let output_name = self.output.evaluate(state)?;
        builder.build(template, output_name, state, names)
    }
}

#[derive(Clone, Debug)]
pub struct BuildObjectExpr {
    pub base: BuildStringExpr,
    pub properties: HashMap<VarNameId, ObjectExpr>,
}

impl BuildObjectExpr {
    pub fn new(base: BuildStringExpr) -> Self {
        Self {
            base,
            properties: HashMap::new(),
        }
    }

    pub fn evaluate<'a>(
        &self,
        state: &mut ProgramState,
        builder: &mut TemplateBuilder<'a>,
        names: &VarNames,
    ) -> Result<Object, TemplateBuildError> {
        let base = self.base.evaluate(state, builder, names)?;
        let mut properties = HashMap::default();

        for (key, value) in self.properties.iter() {
            let value = value.evaluate(state)?;
            properties.insert(*key, value);
        }

        Ok(Object::Struct(Struct { base, properties }))
    }
}

#[derive(Clone, Debug)]
pub enum YieldExpr {
    Build(BuildObjectExpr),
    Object(ObjectExpr),
}

impl YieldExpr {
    pub fn evaluate<'a>(
        &self,
        state: &mut ProgramState,
        builder: &mut TemplateBuilder<'a>,
        names: &VarNames,
    ) -> Result<Object, TemplateBuildError> {
        match self {
            YieldExpr::Build(build_object_expr) => {
                build_object_expr.evaluate(state, builder, names)
            }
            YieldExpr::Object(object_expr) => Ok(object_expr.evaluate(state)?),
        }
    }
}

#[derive(Clone, Debug)]
pub enum TemplateCommand {
    BuildAssign {
        output: VarNameId,
        object: BuildObjectExpr,
    },
    Yield {
        output: VarNameId,
        object: YieldExpr,
    },
}
