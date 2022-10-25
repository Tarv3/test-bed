use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use pest::{iterators::Pair, Parser};

use crate::{
    bed::{
        commands::{ArgBuilder, Command, OutputMap, Spawn},
        expr::{ObjectExpr, StringExpr, StringInstance, VariableExpr},
        templates::{BuildObjectExpr, BuildStringExpr, TemplateCommand},
    },
    program::{Instruction, InstructionId, Program, VarFieldId, VarNameId, VarNames},
};

use self::{commands::build_commands_program, templates::build_templates_program};

pub mod commands;
pub mod templates;

#[derive(Parser)]
#[grammar = "grammar.pest"]
pub struct TestBedParser;

pub struct Parsed {
    pub names: VarNames,
    pub includes: Vec<PathBuf>,
    pub output: PathBuf,
    pub globals: Program<TemplateCommand>,
    pub templates: Vec<(VarNameId, Vec<TemplateExpr>)>,
    pub commands: BTreeMap<Option<VarNameId>, Vec<CommandExpr>>,
    // pub commands: Vec<CommandExpr>,
}

impl Parsed {
    pub fn template_program(&self) -> Vec<(String, Program<TemplateCommand>)> {
        self.templates
            .clone()
            .drain(..)
            .map(|(id, value)| {
                let name = self.names.evaluate(id).unwrap().to_string();
                let program = build_templates_program(value.into_iter());
                (name, program)
            })
            .collect()
    }

    pub fn commands_program(
        &self,
        name: Option<VarNameId>,
    ) -> Option<(Option<String>, Program<Command>)> {
        let commands = self.commands.get(&name)?.clone();
        let name = name.map(|value| self.names.evaluate(value).unwrap().to_string());
        Some((name, build_commands_program(commands.into_iter())))
    }

    pub fn all_programs(&self) -> Vec<(Option<String>, Program<Command>)> {
        self.commands
            .clone()
            .into_iter()
            .map(|(id, program)| {
                let name = id.map(|value| self.names.evaluate(value).unwrap().to_string());
                let program = build_commands_program(program.into_iter());
                (name, program)
            })
            .collect()
    }
}

#[derive(Clone)]
pub struct ForLoop {
    pub ty: ForLoopType,
    pub iters: Vec<VarNameId>,
    pub targets: Vec<VarNameId>,
}

pub fn build_group_loop<T>(
    iters: &[VarNameId],
    targets: &[VarNameId],
    instructions: &mut Vec<Instruction<T>>,
    f: impl FnOnce(&mut Vec<Instruction<T>>),
) {
    instructions.push(Instruction::PushScope);
    let iter_start = instructions.len();

    for (iter, target) in iters.iter().zip(targets.iter()) {
        instructions.push(Instruction::StartIter {
            target: *target,
            iter: *iter,
            end: InstructionId(0),
        })
    }

    let goto = instructions.len();

    instructions.push(Instruction::PushScope);
    f(instructions);
    instructions.push(Instruction::PopScope);

    let increment_start = instructions.len();

    for (iter, target) in iters.iter().zip(targets.iter()) {
        instructions.push(Instruction::Increment {
            target: *target,
            iter: *iter,
            end: InstructionId(0),
        })
    }

    instructions.push(Instruction::Goto(InstructionId(goto)));
    let end_idx = instructions.len();
    instructions.push(Instruction::PopScope);

    for i in iter_start..goto {
        if let Instruction::StartIter { end, .. } = &mut instructions[i] {
            *end = InstructionId(end_idx)
        }
    }

    for i in increment_start..end_idx {
        if let Instruction::Increment { end, .. } = &mut instructions[i] {
            *end = InstructionId(end_idx)
        }
    }
}

pub fn build_combination_loop<T>(
    iters: &[VarNameId],
    targets: &[VarNameId],
    instructions: &mut Vec<Instruction<T>>,
    f: impl FnOnce(&mut Vec<Instruction<T>>),
) {
    if iters.is_empty() {
        instructions.push(Instruction::PushScope);
        f(instructions);
        instructions.push(Instruction::PopScope);
        return;
    }

    let this_iter = iters[0];
    let this_target = targets[0];
    let remaining_iters = &iters[1..];
    let remaining_targets = &targets[1..];

    instructions.push(Instruction::PushScope);
    let iter_start = instructions.len();
    instructions.push(Instruction::StartIter {
        target: this_target,
        iter: this_iter,
        end: InstructionId(0),
    });

    let goto = instructions.len();

    build_combination_loop(remaining_iters, remaining_targets, instructions, f);

    let increment_start = instructions.len();
    instructions.push(Instruction::Increment {
        target: this_target,
        iter: this_iter,
        end: InstructionId(0),
    });

    instructions.push(Instruction::Goto(InstructionId(goto)));
    let end_idx = instructions.len();
    instructions.push(Instruction::PopScope);

    if let Instruction::StartIter { end, .. } = &mut instructions[iter_start] {
        *end = InstructionId(end_idx)
    }
    if let Instruction::Increment { end, .. } = &mut instructions[increment_start] {
        *end = InstructionId(end_idx)
    }
}

impl ForLoop {
    pub fn build<T>(
        &self,
        instructions: &mut Vec<Instruction<T>>,
        f: impl FnOnce(&mut Vec<Instruction<T>>),
    ) {
        match self.ty {
            ForLoopType::Group => build_group_loop(&self.iters, &self.targets, instructions, f),
            ForLoopType::Combinations => {
                build_combination_loop(&self.iters, &self.targets, instructions, f)
            }
        }
    }
}

#[derive(Clone, Debug)]
pub enum ForLoopType {
    Group,
    Combinations,
}

pub fn parse_test_bed(file: impl AsRef<Path>) -> Parsed {
    let file = std::fs::read_to_string(file).unwrap();
    let ast = TestBedParser::parse(Rule::main, &file).unwrap();
    let mut variables = VarNames::default();
    let mut globals = Program(vec![]);
    let mut templates = vec![];
    let mut commands = BTreeMap::new();
    let mut includes = vec![];
    let mut output = PathBuf::new();

    for value in ast {
        match value.as_rule() {
            Rule::includes => {
                let inner = value.into_inner();
                for value in inner {
                    let inner = value.into_inner().next().unwrap();
                    includes.push(PathBuf::from(inner.as_str()));
                }
            }
            Rule::template_output => {
                let inner = value
                    .into_inner()
                    .next()
                    .unwrap()
                    .into_inner()
                    .next()
                    .unwrap();
                output = PathBuf::from(inner.as_str());
            }
            Rule::globals => {
                let inner = value.into_inner().next().unwrap();
                globals = parse_globals_program(&mut variables, inner);
            }
            Rule::templates => {
                let mut inner = value.into_inner();
                let ident = inner.next().unwrap();
                let ident = parse_ident(&mut variables, ident);
                let program = inner.next().unwrap();
                let program = parse_template_program(ident, &mut variables, program);

                templates.push((ident, program))
            }
            Rule::commands => {
                let mut inner = value.into_inner();
                let next = inner.next().unwrap();

                let (ident, program) = match next.as_rule() {
                    Rule::ident => {
                        let ident = parse_ident(&mut variables, next);
                        let program = inner.next().unwrap();
                        let program = parse_command_program(&mut variables, program);
                        (Some(ident), program)
                    }
                    Rule::command_program => {
                        let program = parse_command_program(&mut variables, next);
                        (None, program)
                    }
                    _ => unreachable!(),
                };

                commands.insert(ident, program);
            }
            Rule::EOI => break,
            _ => {
                unreachable!()
            }
        }
    }

    Parsed {
        names: variables,
        globals,
        templates,
        commands,
        includes,
        output,
    }
}

// ======================= Globals ===========================

pub fn parse_globals_program<T>(variables: &mut VarNames, pair: Pair<Rule>) -> Program<T> {
    let inner = pair.into_inner();
    let mut exprs = vec![];

    for value in inner {
        let (target, value) = parse_variable_assignment(variables, value);

        let instruction = Instruction::AssignVar {
            target,
            scope: None,
            value,
        };

        exprs.push(instruction);
    }

    Program(exprs)
}

// ======================= Globals ===========================

// ======================= Templates ===========================

#[derive(Clone)]
pub enum TemplateExpr {
    Command(Instruction<TemplateCommand>),
    ForLoop {
        for_loop: ForLoop,
        exprs: Vec<TemplateExpr>,
    },
}

pub fn parse_template_program(
    template_target: VarNameId,
    variables: &mut VarNames,
    pair: Pair<Rule>,
) -> Vec<TemplateExpr> {
    let inner = pair.into_inner();
    let mut exprs = vec![];

    for value in inner {
        exprs.push(parse_template_expr(template_target, variables, value));
    }

    exprs
}

pub fn parse_template_expr(
    template_target: VarNameId,
    variables: &mut VarNames,
    pair: Pair<Rule>,
) -> TemplateExpr {
    let inner = pair.into_inner().next().unwrap();

    match inner.as_rule() {
        Rule::template => {
            let command = parse_template(template_target, variables, inner);
            TemplateExpr::Command(command)
        }
        Rule::template_for_loop => {
            let mut inner = inner.into_inner();
            let for_loop = inner.next().unwrap();
            let for_loop = parse_for_loop(variables, for_loop);

            let mut exprs = vec![];

            for value in inner {
                let expr = parse_template_expr(template_target, variables, value);
                exprs.push(expr);
            }

            TemplateExpr::ForLoop { for_loop, exprs }
        }
        _ => {
            unreachable!()
        }
    }
}

pub fn parse_template(
    template_target: VarNameId,
    variables: &mut VarNames,
    pair: Pair<Rule>,
) -> Instruction<TemplateCommand> {
    let inner = pair.into_inner().next().unwrap();

    match inner.as_rule() {
        Rule::build_assignment => {
            let (output, object) = parse_build_assignment(variables, inner);
            Instruction::Command(TemplateCommand::BuildAssign { output, object })
        }
        Rule::yield_template => {
            let yield_object = parse_yield_template(variables, inner);

            Instruction::Command(TemplateCommand::Yield {
                output: template_target,
                object: yield_object,
            })
        }
        _ => unreachable!(),
    }
}

pub fn parse_build_assignment(
    variables: &mut VarNames,
    pair: Pair<Rule>,
) -> (VarNameId, BuildObjectExpr) {
    let mut inner = pair.into_inner();
    let ident = parse_ident(variables, inner.next().unwrap());
    let object = parse_build_object(variables, inner.next().unwrap());

    (ident, object)
}

pub fn parse_yield_template(variables: &mut VarNames, pair: Pair<Rule>) -> BuildObjectExpr {
    let inner = pair.into_inner().next().unwrap();
    parse_build_object(variables, inner)
}

pub fn parse_build_object(variables: &mut VarNames, pair: Pair<Rule>) -> BuildObjectExpr {
    let mut inner = pair.into_inner();
    let base = parse_build_string_expr(variables, inner.next().unwrap());
    let mut object = BuildObjectExpr::new(base);

    for value in inner {
        let (id, expr) = parse_build_property_assignment(variables, value);
        object.properties.insert(id, expr);
    }

    object
}

pub fn parse_build_property_assignment(
    variables: &mut VarNames,
    pair: Pair<Rule>,
) -> (VarNameId, BuildStringExpr) {
    let mut inner = pair.into_inner();
    let ident = parse_ident(variables, inner.next().unwrap());
    let expr = parse_build_string_expr(variables, inner.next().unwrap());

    (ident, expr)
}

pub fn parse_build_string_expr(variables: &mut VarNames, pair: Pair<Rule>) -> BuildStringExpr {
    let inner = pair.into_inner().next().unwrap();

    match inner.as_rule() {
        Rule::string_builder => BuildStringExpr::String(parse_string_builder(variables, inner)),
        Rule::build_fn => {
            let (template, name) = parse_build_fn(variables, inner);
            BuildStringExpr::Build(template, name)
        }
        _ => unreachable!(),
    }
}

pub fn parse_build_fn(variables: &mut VarNames, pair: Pair<Rule>) -> (StringExpr, StringExpr) {
    let mut inner = pair.into_inner();
    let template = inner.next().unwrap();
    let template = parse_string_builder(variables, template);

    let name = inner.next().unwrap();
    let name = parse_string_builder(variables, name);

    (template, name)
}

// pub fn parse_template_object(variables: &mut VarNames, pair: Pair<Rule>) ->

// ======================= Templates ===========================

// ======================= Commands ===========================

#[derive(Clone)]
pub enum CommandExpr {
    Command(Instruction<Command>),
    ForLoop {
        for_loop: ForLoop,
        exprs: Vec<CommandExpr>,
    },
}

pub fn parse_command_program(variables: &mut VarNames, pair: Pair<Rule>) -> Vec<CommandExpr> {
    let inner = pair.into_inner();
    let mut exprs = vec![];

    for value in inner {
        exprs.push(parse_command_expr(variables, value));
    }

    exprs
}

pub fn parse_command_expr(variables: &mut VarNames, pair: Pair<Rule>) -> CommandExpr {
    let inner = pair.into_inner().next().unwrap();

    match inner.as_rule() {
        Rule::command => {
            let command = parse_command(variables, inner);
            CommandExpr::Command(command)
        }
        Rule::command_for_loop => {
            let mut inner = inner.into_inner();
            let for_loop = inner.next().unwrap();
            let for_loop = parse_for_loop(variables, for_loop);

            let mut exprs = vec![];

            for value in inner {
                let expr = parse_command_expr(variables, value);
                exprs.push(expr);
            }

            CommandExpr::ForLoop { for_loop, exprs }
        }
        _ => unreachable!(),
    }
}

pub fn parse_command(variables: &mut VarNames, pair: Pair<Rule>) -> Instruction<Command> {
    let inner = pair.into_inner().next().unwrap();

    match inner.as_rule() {
        Rule::variable_assignment => {
            let (target, value) = parse_variable_assignment(variables, inner);

            Instruction::AssignVar {
                target,
                scope: None,
                value,
            }
        }
        Rule::limit_spawn => {
            let limit = parse_limit_spawn(inner);
            Instruction::Command(Command::LimitSpawn(limit))
        }
        Rule::sleep => {
            let ms = parse_sleep(inner);
            Instruction::Command(Command::Sleep(ms))
        }
        Rule::wait_all => {
            let wait = parse_wait_all(inner);
            Instruction::Command(Command::WaitAll(wait))
        }
        Rule::spawn => {
            let spawn = parse_spawn(variables, inner);
            Instruction::Command(Command::Spawn(spawn))
        }
        _ => unreachable!(),
    }
}

pub fn parse_limit_spawn(pair: Pair<Rule>) -> usize {
    let inner = pair.into_inner().next().unwrap();
    inner.as_str().parse().unwrap()
}

pub fn parse_sleep(pair: Pair<Rule>) -> u64 {
    let inner = pair.into_inner().next().unwrap();
    inner.as_str().parse().unwrap()
}

pub fn parse_wait_all(pair: Pair<Rule>) -> Option<u64> {
    let mut inner = pair.into_inner();
    let mut wait = None;

    if let Some(value) = inner.next() {
        wait = Some(value.as_str().parse().unwrap());
    }

    wait
}

pub fn parse_spawn(variables: &mut VarNames, pair: Pair<Rule>) -> Spawn {
    let mut inner = pair.into_inner();
    let first = inner.next().unwrap();

    let (out, err, command) = match first.as_rule() {
        Rule::std_map => {
            let (out, err) = parse_stdmap(variables, first);
            let next = inner.next().unwrap();
            let command = parse_string_builder(variables, next);

            (out, err, command)
        }
        Rule::string_builder => {
            let command = parse_string_builder(variables, first);
            (OutputMap::Print, OutputMap::Print, command)
        }
        _ => unreachable!(),
    };

    let mut args = vec![];

    for value in inner {
        args.push(parse_arg_builder(variables, value));
    }

    Spawn {
        command,
        args,
        stdout: out,
        stderr: err,
    }
}

pub fn parse_arg_builder(variables: &mut VarNames, pair: Pair<Rule>) -> ArgBuilder {
    let inner = pair.into_inner().next().unwrap();
    match inner.as_rule() {
        Rule::string_builder => ArgBuilder::String(parse_string_builder(variables, inner)),
        Rule::variable_access => ArgBuilder::Set(parse_variable_access(variables, inner)),
        _ => unreachable!(),
    }
}

pub fn parse_stdmap(
    variables: &mut VarNames,
    pair: Pair<Rule>,
) -> (OutputMap<StringExpr>, OutputMap<StringExpr>) {
    let mut inner = pair.into_inner();
    let first = inner.next().unwrap();

    let mut out = OutputMap::Print;
    let mut err = OutputMap::Print;

    match first.as_rule() {
        Rule::stdout_map => {
            let inner = first.into_inner().next().unwrap();
            out = parse_output_map(variables, inner);
        }
        Rule::stderr_map => {
            let inner = first.into_inner().next().unwrap();
            err = parse_output_map(variables, inner);
        }
        _ => unreachable!(),
    }

    if let Some(second) = inner.next() {
        match second.as_rule() {
            Rule::stdout_map => {
                let inner = second.into_inner().next().unwrap();
                out = parse_output_map(variables, inner);
            }
            Rule::stderr_map => {
                let inner = second.into_inner().next().unwrap();
                err = parse_output_map(variables, inner);
            }
            _ => unreachable!(),
        }
    }

    (out, err)
}

pub fn parse_output_map(variables: &mut VarNames, pair: Pair<Rule>) -> OutputMap<StringExpr> {
    let inner = pair.into_inner().next().unwrap();

    match inner.as_rule() {
        Rule::append => {
            let inner = inner.into_inner().next().unwrap();
            let expr = parse_string_builder(variables, inner);

            OutputMap::Append(expr)
        }
        Rule::string_builder => {
            let expr = parse_string_builder(variables, inner);
            OutputMap::Create(expr)
        }
        Rule::print => OutputMap::Print,
        _ => {
            unreachable!()
        }
    }
}

// ======================= Commands ===========================

pub fn parse_for_loop(variables: &mut VarNames, pair: Pair<Rule>) -> ForLoop {
    let inner = pair.into_inner().next().unwrap();

    let ty = match inner.as_rule() {
        Rule::for_loop_combinations => ForLoopType::Combinations,
        Rule::for_loop_groups => ForLoopType::Group,
        _ => unreachable!(),
    };

    let mut inner = inner.into_inner();

    let iters_pairs = inner.next().unwrap();
    let targets_pairs = inner.next().unwrap();
    let iters;
    let targets;

    match iters_pairs.as_rule() {
        Rule::ident => {
            iters = vec![parse_ident(variables, iters_pairs)];
            targets = vec![parse_ident(variables, targets_pairs)];
        }
        Rule::ident_group => {
            iters = parse_ident_group(variables, iters_pairs);
            targets = parse_ident_group(variables, targets_pairs);
        }
        _ => unreachable!(),
    }

    ForLoop { ty, iters, targets }
}

pub fn parse_ident_group(variables: &mut VarNames, pair: Pair<Rule>) -> Vec<VarNameId> {
    let mut group = vec![];
    let inner = pair.into_inner();

    for value in inner {
        group.push(parse_ident(variables, value));
    }

    group
}

pub fn parse_variable_assignment(
    variables: &mut VarNames,
    pair: Pair<Rule>,
) -> (VarNameId, VariableExpr) {
    let inner = pair.into_inner().next().unwrap();

    match inner.as_rule() {
        Rule::list_assignment => {
            let (target, list) = parse_list_assignment(variables, inner);
            (target, VariableExpr::List(list))
        }
        Rule::object_assignment => {
            let (target, object) = parse_object_assignment(variables, inner);
            (target, VariableExpr::Object(object))
        }
        _ => {
            unreachable!()
        }
    }
}

pub fn parse_list_assignment(
    variables: &mut VarNames,
    pair: Pair<Rule>,
) -> (VarNameId, Vec<ObjectExpr>) {
    let mut inner = pair.into_inner();
    let ident = inner.next().unwrap();
    let ident = parse_ident(variables, ident);
    let mut objects = vec![];

    for value in inner {
        objects.push(parse_object(variables, value));
    }

    (ident, objects)
}

pub fn parse_object_assignment(
    variables: &mut VarNames,
    pair: Pair<Rule>,
) -> (VarNameId, ObjectExpr) {
    let mut inner = pair.into_inner();

    let ident = inner.next().unwrap();
    let ident = parse_ident(variables, ident);

    let object = inner.next().unwrap();
    let object = parse_object(variables, object);

    (ident, object)
}

pub fn parse_object(variables: &mut VarNames, pair: Pair<Rule>) -> ObjectExpr {
    let mut inner = pair.into_inner();

    let base = inner.next().unwrap();
    let base = parse_string_builder(variables, base);
    let mut object = ObjectExpr::new(base);

    for value in inner {
        let (id, value) = parse_property_assignment(variables, value);
        object.properties.insert(id, value);
    }

    object
}

pub fn parse_property_assignment(
    variables: &mut VarNames,
    pair: Pair<Rule>,
) -> (VarNameId, StringExpr) {
    let mut inner = pair.into_inner();

    let ident = inner.next().unwrap();
    let ident = parse_ident(variables, ident);

    let string_builder = inner.next().unwrap();
    let expr = parse_string_builder(variables, string_builder);

    (ident, expr)
}

pub fn parse_ident(variables: &mut VarNames, pair: Pair<Rule>) -> VarNameId {
    let ident = pair.as_str();
    let ident = variables.replace(ident);

    ident
}

pub fn parse_string_builder(variables: &mut VarNames, pair: Pair<Rule>) -> StringExpr {
    let inner = pair.into_inner();
    let mut expr = StringExpr::default();

    for value in inner {
        let instance = parse_string_instance(variables, value);
        expr.0.push(instance);
    }

    expr
}

pub fn parse_string_instance(variables: &mut VarNames, pair: Pair<Rule>) -> StringInstance {
    let inner = pair.into_inner().next().unwrap();

    match inner.as_rule() {
        Rule::string_no_whitespace => StringInstance::String(inner.as_str().replace("\\\"", "\"")),
        Rule::string_whitespace => {
            let inner = inner.into_inner().next().unwrap();
            StringInstance::String(inner.as_str().replace("\\\"", "\""))
        }
        Rule::variable_access => {
            let field_id = parse_variable_access(variables, inner);
            StringInstance::Variable(field_id)
        }
        _ => unreachable!(),
    }
}

pub fn parse_variable_access(variables: &mut VarNames, pair: Pair<Rule>) -> VarFieldId {
    let mut inner = pair.into_inner();
    let variable = inner.next().unwrap();
    let variable = parse_ident(variables, variable);
    let mut access = VarFieldId::new(variable);

    if let Some(value) = inner.next() {
        let field = parse_ident(variables, value);
        access.field = Some(field);
    }

    access
}
