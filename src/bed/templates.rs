use std::{collections::HashMap, fmt::Display, io::ErrorKind, path::PathBuf};

use minijinja::{Environment, Source};

use crate::program::{Object, ProgramState, VarNameId, VarNames, Variable};

use super::expr::StringExpr;

#[derive(Debug)]
pub enum TemplateErrorType {
    InvalidPath(PathBuf),
    WriteError(std::io::Error),
    RenderError(minijinja::Error),
}

pub struct TemplateBuildError {
    pub template_path: String,
    pub output_path: String,
    pub error: TemplateErrorType,
}

impl Display for TemplateBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Template `{}` -> `{}` ",
            self.template_path, self.output_path
        )?;

        match &self.error {
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

pub struct TemplateBuilder<'source> {
    pub environment: Environment<'source>,
    current_params: HashMap<String, String>,
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
            current_params: HashMap::new(),
            output,
        }
    }

    pub fn build(
        &mut self,
        template_path: String,
        output_name: String,
        state: &mut ProgramState,
        names: &VarNames,
    ) -> Result<String, TemplateBuildError> {
        let template = match self.environment.get_template(&template_path) {
            Ok(template) => template,
            Err(e) => {
                todo!("Template Error: {e}")
            }
        };

        self.current_params.clear();

        for scope in state.scopes.iter().rev() {
            for (name, value) in scope.0.iter() {
                let name = match names.evaluate(*name) {
                    Some(name) => name,
                    None => continue,
                };

                if self.current_params.contains_key(name) {
                    continue;
                }

                let value = match value {
                    crate::program::Variable::Counter(value) => format!("{}", value.idx()),
                    crate::program::Variable::List(_) => continue,
                    crate::program::Variable::Ref(value) => match state.evaluate_ref(*value) {
                        Some(object) => object.base.clone(),
                        None => continue,
                    },
                    crate::program::Variable::Object(object) => object.base.clone(),
                };

                self.current_params.insert(name.into(), value);
            }
        }

        let mut output_file = self.output.clone();
        output_file.push(output_name);

        let output_path = match output_file.to_str() {
            Some(file) => file.to_string(),
            None => {
                return Err(TemplateBuildError {
                    template_path,
                    output_path: output_file.to_string_lossy().to_string(),
                    error: TemplateErrorType::InvalidPath(output_file),
                })
            }
        };

        let rendered = match template.render(&self.current_params) {
            Ok(rendered) => rendered,
            Err(e) => {
                return Err(TemplateBuildError {
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
                    return Err(TemplateBuildError {
                        template_path,
                        output_path,
                        error: TemplateErrorType::WriteError(e),
                    })
                }
            }
        }

        if let Err(e) = std::fs::write(&output_file, rendered) {
            return Err(TemplateBuildError {
                template_path,
                output_path,
                error: TemplateErrorType::WriteError(e),
            });
        };

        Ok(output_path)
    }
}

pub fn yield_value(output: VarNameId, to_yield: Object, state: &mut ProgramState) {
    let output_var = match state.scopes[0].0.get_mut(&output) {
        Some(Variable::List(list)) => list,
        _ => state.new_list(output, Some(0)),
    };

    output_var.push(to_yield)
}

#[derive(Clone, Debug)]
pub enum BuildStringExpr {
    Build(StringExpr, StringExpr),
    String(StringExpr),
}

impl BuildStringExpr {
    pub fn evaluate<'a>(
        &self,
        state: &mut ProgramState,
        builder: &mut TemplateBuilder<'a>,
        names: &VarNames,
    ) -> Result<String, TemplateBuildError> {
        match self {
            BuildStringExpr::Build(template, name) => {
                let template = template.evaluate(state);
                let output_name = name.evaluate(state);
                builder.build(template, output_name, state, names)
            }
            BuildStringExpr::String(string) => Ok(string.evaluate(state)),
        }
    }
}

#[derive(Clone, Debug)]
pub struct BuildObjectExpr {
    pub base: BuildStringExpr,
    pub properties: HashMap<VarNameId, StringExpr>,
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

        let mut object = Object::new(base);
        for (key, value) in self.properties.iter() {
            let value = value.evaluate(state);
            object.properties.insert(*key, value);
        }

        Ok(object)
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
        object: BuildObjectExpr,
    },
}
