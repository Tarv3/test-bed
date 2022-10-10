use crate::{
    bed::templates::TemplateCommand,
    program::{Instruction, Program},
};

use super::TemplateExpr;

pub fn build_templates_program(
    exprs: impl Iterator<Item = TemplateExpr>,
) -> Program<TemplateCommand> {
    let mut instructions = vec![];

    for value in exprs {
        build_expr(value, &mut instructions);
    }

    Program(instructions)
}

pub fn build_expr(expr: TemplateExpr, instructions: &mut Vec<Instruction<TemplateCommand>>) {
    match expr {
        TemplateExpr::Command(command) => instructions.push(command),
        TemplateExpr::ForLoop { for_loop, exprs } => {
            for_loop.build(instructions, |instructions| {
                for expr in exprs {
                    build_expr(expr, instructions);
                }
            });
        }
    }
}
