use crate::{
    bed::commands::Command,
    program::{Instruction, InstructionId, Program},
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
        CommandExpr::If { conditions, exprs } => {
            let start = instructions.len();

            for cond in conditions {
                instructions.push(Instruction::ConditionalJump {
                    cond,
                    jump: InstructionId(0),
                });
            }

            let end = instructions.len();
            instructions.push(Instruction::PushScope);

            for expr in exprs {
                build_expr(expr, instructions);
            }

            instructions.push(Instruction::PopScope);
            let jump_target = instructions.len();

            for i in start..end {
                let Instruction::ConditionalJump { jump, .. } = &mut instructions[i] else { 
                    unreachable!() 
                };

                jump.0 = jump_target;
            }
        }
    }
}
