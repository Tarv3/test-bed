use crate::{
    bed::commands::Command,
    program::{Instruction, Program},
};

use super::CommandExpr;

pub fn build_commands_program(exprs: impl Iterator<Item = CommandExpr>) -> Program<Command> {
    let mut instructions = vec![];

    for value in exprs {
        build_expr(value, &mut instructions);
    }

    Program(instructions)
}

pub fn build_expr(expr: CommandExpr, instructions: &mut Vec<Instruction<Command>>) {
    match expr {
        CommandExpr::Command(command) => instructions.push(command),
        CommandExpr::ForLoop { for_loop, exprs } => {
            for_loop.build(instructions, |instructions| {
                for expr in exprs {
                    build_expr(expr, instructions);
                }
            });
        }
    }
}
