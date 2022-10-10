use std::{collections::HashMap, path::PathBuf};

use minijinja::{Environment, Source};

use crate::program::{Object, ProgramState, VarNameId, VarNames, Variable};

use super::expr::ObjectExpr;

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
        output: VarNameId,
        mut object: Object,
        state: &mut ProgramState,
        names: &VarNames,
    ) {
        let template = match self.environment.get_template(&object.base) {
            Ok(template) => template,
            Err(e) => {
                todo!("Template Error: {e}") 
            },
        };

        let output_name = match names.evaluate(output) {
            Some(name) => name,
            None => unreachable!(),
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
                    crate::program::Variable::List(_) => continue,
                    crate::program::Variable::Ref(value) => match state.evaluate_ref(*value) {
                        Some(object) => object,
                        None => continue,
                    },
                    crate::program::Variable::Object(object) => object,
                };

                self.current_params.insert(name.into(), value.base.clone());
            }
        }

        let output_var = match state.scopes[0].0.get_mut(&output) {
            Some(Variable::List(list)) => list,
            _ => state.new_list(output, Some(0)),
        };

        let id = output_var.len();
        let mut output_file = self.output.clone();
        output_file.push(format!("{output_name}_{id}.j2"));

        let output_file = match output_file.into_os_string().into_string() {
            Ok(value) => value,
            Err(_) => todo!(),
        };

        let rendered = match template.render(&self.current_params) {
            Ok(rendered) => rendered,
            Err(_) => todo!(),
        };

        if let Err(_) = std::fs::write(&output_file, rendered) {
            todo!("Handle file create error")
        };

        object.base = output_file;
        output_var.push(object);
    }
}

#[derive(Clone, Debug)]
pub enum TemplateCommand {
    Yield {
        output: VarNameId,
        object: ObjectExpr,
    },
}
