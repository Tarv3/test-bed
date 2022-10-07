use std::collections::HashMap;

use indicatif::MultiProgress;
use minijinja::Environment;

use crate::bed::Stack;

pub struct TemplateInfo {
    
}

pub struct Templates<'source> {
    pub environment: Environment<'source>,
    pub templates: Vec<String>,
    context: HashMap<String, String>,
}

impl<'source> Templates<'source> {
    pub fn render(
        &mut self,
        template: &str,
        stack: Stack,
        params: &HashMap<String, Vec<String>>,
        bar: &MultiProgress,
    ) {
        self.context.clear();

        let template = match self.environment.get_template(template) {
            Ok(template) => template,
            Err(e) => {
                bar.println(format!("Missing Template: {e}")).unwrap();
                return;
            }
        };

        for idx in stack.0.iter().rev() {
            if self.context.contains_key(&idx.id) {
                continue;
            }

            let param = params
                .get(&idx.param)
                .map(|value| value.get(idx.idx))
                .flatten();

            if let Some(value) = param {
                self.context.insert(idx.id.clone(), value.clone());
            }
        }

        template.render(ctx)
    }
}
