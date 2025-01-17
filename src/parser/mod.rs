use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
};

use pest::{iterators::Pair, Parser};

use crate::{
    bed::{
        commands::{ArgBuilder, Command, OutputMap, Spawn},
        expr::{IterTargetExpr, ObjectExpr, RangeExpr, StringExpr, StringInstance, StructExpr},
        templates::{BuildObjectExpr, BuildStringExpr, TemplateCommand, YieldExpr},
    },
    program::{Instruction, InstructionId, Program, VarFieldId, VarNameId, VarNames, VariableIdx},
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
    pub targets: Vec<IterTargetExpr>,
}

pub fn build_group_loop<T>(
    iters: &[VarNameId],
    targets: &[IterTargetExpr],
    instructions: &mut Vec<Instruction<T>>,
    f: impl FnOnce(&mut Vec<Instruction<T>>),
) {
    instructions.push(Instruction::PushScope);
    let iter_start = instructions.len();

    for (iter, target) in iters.iter().zip(targets.iter()) {
        instructions.push(Instruction::StartIter {
            target: target.clone(),
            iter: *iter,
            jump: InstructionId(0),
        })
    }

    let goto = instructions.len();

    instructions.push(Instruction::PushScope);
    f(instructions);
    instructions.push(Instruction::PopScope);

    let increment_start = instructions.len();

    for (iter, target) in iters.iter().zip(targets.iter()) {
        instructions.push(Instruction::Increment {
            target: target.to_itertarget(),
            iter: *iter,
            jump: InstructionId(0),
        })
    }

    instructions.push(Instruction::Goto(InstructionId(goto)));
    let end_idx = instructions.len();
    instructions.push(Instruction::PopScope);

    for i in iter_start..goto {
        if let Instruction::StartIter { jump, .. } = &mut instructions[i] {
            *jump = InstructionId(end_idx)
        }
    }

    for i in increment_start..end_idx {
        if let Instruction::Increment { jump, .. } = &mut instructions[i] {
            *jump = InstructionId(end_idx)
        }
    }
}

pub fn build_combination_loop<T>(
    iters: &[VarNameId],
    targets: &[IterTargetExpr],
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
    let this_target = &targets[0];
    let remaining_iters = &iters[1..];
    let remaining_targets = &targets[1..];

    instructions.push(Instruction::PushScope);
    let iter_start = instructions.len();
    instructions.push(Instruction::StartIter {
        target: this_target.clone(),
        iter: this_iter,
        jump: InstructionId(0),
    });

    let goto = instructions.len();

    build_combination_loop(remaining_iters, remaining_targets, instructions, f);

    let increment_start = instructions.len();
    instructions.push(Instruction::Increment {
        target: this_target.to_itertarget(),
        iter: this_iter,
        jump: InstructionId(0),
    });

    instructions.push(Instruction::Goto(InstructionId(goto)));
    let end_idx = instructions.len();
    instructions.push(Instruction::PopScope);

    if let Instruction::StartIter { jump, .. } = &mut instructions[iter_start] {
        *jump = InstructionId(end_idx)
    }
    if let Instruction::Increment { jump, .. } = &mut instructions[increment_start] {
        *jump = InstructionId(end_idx)
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
        let instruction = parse_variable_assignment(variables, value);
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
    If {
        conditions: Vec<VarFieldId>,
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
        Rule::template_if_statement => {
            let mut inner = inner.into_inner();
            let if_statement = inner.next().unwrap();
            let conditions = parse_if_statement(variables, if_statement);

            let mut exprs = vec![];

            for value in inner {
                let expr = parse_template_expr(template_target, variables, value);
                exprs.push(expr);
            }

            TemplateExpr::If { conditions, exprs }
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
        Rule::variable_assignment => parse_variable_assignment(variables, inner),
        Rule::push => {
            let (target, object) = parse_push(variables, inner);
            Instruction::PushList { target, object }
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

pub fn parse_yield_template(variables: &mut VarNames, pair: Pair<Rule>) -> YieldExpr {
    let inner = pair.into_inner().next().unwrap();
    parse_yield_object(variables, inner)
}

pub fn parse_yield_object(variables: &mut VarNames, pair: Pair<Rule>) -> YieldExpr {
    let inner = pair.into_inner().next().unwrap();

    match inner.as_rule() {
        Rule::build_object => YieldExpr::Build(parse_build_object(variables, inner)),
        Rule::object => YieldExpr::Object(parse_object_expr(variables, inner)),
        x => unreachable!("{x:?}"),
    }
}

pub fn parse_build_object(variables: &mut VarNames, pair: Pair<Rule>) -> BuildObjectExpr {
    let mut inner = pair.into_inner();
    let base = parse_build_fn(variables, inner.next().unwrap());
    let mut object = BuildObjectExpr::new(base);

    for value in inner {
        let (id, expr) = parse_property_assignment(variables, value);
        object.properties.insert(id, expr);
    }

    object
}

pub fn parse_build_fn(variables: &mut VarNames, pair: Pair<Rule>) -> BuildStringExpr {
    let mut inner = pair.into_inner();
    let template = inner.next().unwrap();
    let template = parse_string_builder(variables, template);

    let name = inner.next().unwrap();
    let name = parse_string_builder(variables, name);

    BuildStringExpr {
        template,
        output: name,
    }
}

// ======================= Templates ===========================

// ======================= Commands ===========================

#[derive(Clone)]
pub enum CommandExpr {
    Command(Instruction<Command>),
    ForLoop {
        for_loop: ForLoop,
        exprs: Vec<CommandExpr>,
    },
    If {
        conditions: Vec<VarFieldId>,
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
        Rule::command_if_statement => {
            let mut inner = inner.into_inner();
            let if_statement = inner.next().unwrap();
            let conditions = parse_if_statement(variables, if_statement);

            let mut exprs = vec![];

            for value in inner {
                let expr = parse_command_expr(variables, value);
                exprs.push(expr);
            }

            CommandExpr::If { conditions, exprs }
        }
        _ => unreachable!(),
    }
}

pub fn parse_command(variables: &mut VarNames, pair: Pair<Rule>) -> Instruction<Command> {
    let inner = pair.into_inner().next().unwrap();

    match inner.as_rule() {
        Rule::variable_assignment => parse_variable_assignment(variables, inner),
        Rule::push => {
            let (target, object) = parse_push(variables, inner);
            Instruction::PushList { target, object }
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

    let mut working_dir = None;
    let mut out = OutputMap::Print;
    let mut err = OutputMap::Print;

    let mut next = inner.next().unwrap();

    while next.as_rule() != Rule::string_builder {
        match next.as_rule() {
            Rule::working_dir => {
                working_dir = Some(parse_working_dir(variables, next));
            }
            Rule::std_map => {
                (out, err) = parse_stdmap(variables, next);
            }
            _ => unreachable!(),
        }

        next = inner.next().unwrap();
    }

    let command = parse_string_builder(variables, next);
    let mut args = vec![];

    for value in inner {
        args.push(parse_arg_builder(variables, value));
    }

    Spawn {
        command,
        working_dir,
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

pub fn parse_working_dir(variables: &mut VarNames, pair: Pair<Rule>) -> StringExpr {
    let mut inner = pair.into_inner();
    let inner = inner.next().unwrap();

    parse_string_builder(variables, inner)
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

pub fn parse_if_statement(variables: &mut VarNames, pair: Pair<Rule>) -> Vec<VarFieldId> {
    let mut conditions = vec![];
    let inner = pair.into_inner();

    for value in inner {
        let access = parse_variable_access(variables, value);
        conditions.push(access);
    }

    conditions
}

pub fn parse_for_loop(variables: &mut VarNames, pair: Pair<Rule>) -> ForLoop {
    let inner = pair.into_inner().next().unwrap();
    let (line, col) = inner.line_col();

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
            targets = vec![parse_iterable(variables, targets_pairs)];
        }
        Rule::ident_group => {
            iters = parse_ident_group(variables, iters_pairs);
            targets = parse_iterable_group_group(variables, targets_pairs);
        }
        _ => unreachable!(),
    }

    if iters.len() != targets.len() {
        panic!(
            "Incorrect number of iter variables: [Line {}, Column {}]",
            line, col
        );
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

pub fn parse_iterable_group_group(
    variables: &mut VarNames,
    pair: Pair<Rule>,
) -> Vec<IterTargetExpr> {
    let mut group = vec![];
    let inner = pair.into_inner();

    for value in inner {
        group.push(parse_iterable(variables, value));
    }

    group
}

pub fn parse_iterable(variables: &mut VarNames, pair: Pair<Rule>) -> IterTargetExpr {
    let inner = pair.into_inner().next().unwrap();

    match inner.as_rule() {
        Rule::ident => {
            let ident = parse_ident(variables, inner);

            IterTargetExpr::Variable(ident)
        }
        Rule::range => {
            let (start, end) = parse_range(variables, inner);
            IterTargetExpr::Range { start, end }
        }
        _ => {
            unreachable!()
        }
    }
}

pub fn parse_range(variables: &mut VarNames, pair: Pair<Rule>) -> (RangeExpr, RangeExpr) {
    let mut iter = pair.into_inner();
    let start = iter.next().unwrap();
    let end = iter.next().unwrap();

    let start = parse_range_expr(variables, start);
    let end = parse_range_expr(variables, end);

    (start, end)
}

pub fn parse_range_expr(variables: &mut VarNames, pair: Pair<Rule>) -> RangeExpr {
    let inner = pair.into_inner().next().unwrap();

    match inner.as_rule() {
        Rule::variable_access => {
            let field_id = parse_variable_access(variables, inner);
            let var = StringInstance::Variable(field_id);
            let expr = StringExpr(vec![var]);
            RangeExpr::Variable(expr)
        }
        Rule::signed_integer => {
            let value = parse_signed_integer(inner);
            RangeExpr::Integer(value)
        }
        _ => unreachable!(),
    }
}

pub fn parse_signed_integer(pair: Pair<Rule>) -> i64 {
    // let mut iter = pair.into_inner();
    // let value = iter.next().unwrap();
    let value = pair;

    let (value_line, value_col) = value.line_col();
    let Ok(value) = value.as_str().parse() else {
        panic!(
            "Failed to parse value `{}`: [Line {}, Column {}]",
            value.as_str(),
            value_line,
            value_col
        );
    };

    value
}

pub fn parse_variable_assignment<T>(variables: &mut VarNames, pair: Pair<Rule>) -> Instruction<T> {
    let mut inner = pair.into_inner();
    let ident = parse_ident(variables, inner.next().unwrap());
    let create = parse_variable_assign_op(inner.next().unwrap());
    let expr = parse_object_expr(variables, inner.next().unwrap());

    match create {
        true => Instruction::CreateVar {
            target: ident,
            scope: None,
            value: expr,
        },
        false => Instruction::AssignVar {
            target: ident,
            scope: None,
            value: expr,
        },
    }
}

// Returns true if the op is create
pub fn parse_variable_assign_op(pair: Pair<Rule>) -> bool {
    let inner = pair.into_inner().next().unwrap();

    match inner.as_rule() {
        Rule::variable_create => true,
        Rule::variable_assign => false,
        _ => unreachable!(),
    }
}

pub fn parse_list_expression(variables: &mut VarNames, pair: Pair<Rule>) -> Vec<ObjectExpr> {
    let inner = pair.into_inner();
    let mut objects = vec![];

    for value in inner {
        objects.push(parse_object_expr(variables, value));
    }

    objects
}

pub fn parse_push(variables: &mut VarNames, pair: Pair<Rule>) -> (VarNameId, ObjectExpr) {
    let mut inner = pair.into_inner();
    let ident = inner.next().unwrap();
    let ident = parse_ident(variables, ident);

    let object = inner.next().unwrap();
    let object = parse_object_expr(variables, object);

    (ident, object)
}

pub fn parse_object_expr(variables: &mut VarNames, pair: Pair<Rule>) -> ObjectExpr {
    let mut inner = pair.into_inner();
    let inner = inner.next().unwrap();

    let object = match inner.as_rule() {
        Rule::variable_clone => ObjectExpr::Clone(parse_variable_clone(variables, inner)),
        Rule::list_expression => ObjectExpr::List(parse_list_expression(variables, inner)),
        Rule::struct_expr => ObjectExpr::Struct(parse_struct_expression(variables, inner)),
        Rule::range => {
            let (min, max) = parse_range(variables, inner);
            ObjectExpr::Counter(min, max)
        }
        x => unreachable!("{x:?}"),
    };

    object
}

pub fn parse_struct_expression(variables: &mut VarNames, pair: Pair<Rule>) -> StructExpr {
    let mut inner = pair.into_inner();
    let base = inner.next().unwrap();
    let base = parse_string_builder(variables, base);
    let mut properties = HashMap::new();

    for value in inner {
        let (name, expr) = parse_property_assignment(variables, value);
        properties.insert(name, expr);
    }

    StructExpr { base, properties }
}

pub fn parse_variable_clone(variables: &mut VarNames, pair: Pair<Rule>) -> VarFieldId {
    let mut inner = pair.into_inner();
    let base = inner.next().unwrap();
    parse_variable_access(variables, base)
}

pub fn parse_property_assignment(
    variables: &mut VarNames,
    pair: Pair<Rule>,
) -> (VarNameId, ObjectExpr) {
    let mut inner = pair.into_inner();

    let ident = inner.next().unwrap();
    let ident = parse_ident(variables, ident);

    let object = inner.next().unwrap();
    let expr = parse_object_expr(variables, object);

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
    let mut access = parse_immediate_variable_access(variables, variable);

    if let Some(value) = inner.next() {
        access.field = Some(Box::new(parse_variable_access(variables, value)))
    }

    access
}

pub fn parse_immediate_variable_access(variables: &mut VarNames, pair: Pair<Rule>) -> VarFieldId {
    let mut inner = pair.into_inner();
    let variable = inner.next().unwrap();
    let ident = parse_ident(variables, variable);
    let mut access = VarFieldId::new(ident);

    if let Some(value) = inner.next() {
        access.idx = Some(Box::new(parse_variable_idx(variables, value)))
    }

    access
}

pub fn parse_variable_idx(variables: &mut VarNames, pair: Pair<Rule>) -> VariableIdx {
    let mut inner = pair.into_inner();
    let idx = inner.next().unwrap();

    match idx.as_rule() {
        Rule::integer => {
            let idx = idx.as_str().parse().unwrap();
            VariableIdx::Integer(idx)
        }
        Rule::variable_access => {
            let access = parse_variable_access(variables, idx);
            VariableIdx::Variable(access)
        }
        _ => unreachable!(),
    }
}
