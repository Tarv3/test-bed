use std::{path::PathBuf, time::Duration};

use indicatif::{MultiProgress, ProgressDrawTarget};

use crate::program::{Executable, ProgramState, VarNames};

use self::{
    commands::Command,
    process::ProcessInfo,
    templates::{TemplateBuilder, TemplateCommand},
};

pub mod commands;
pub mod expr;
pub mod process;
pub mod templates;

pub const SLEEP_TIME: Duration = Duration::from_millis(100);

pub struct TestBed<'source> {
    pub templates: TemplateBuilder<'source>,
    pub var_names: VarNames,

    pub spawn_limit: Option<usize>,
    pub processes: Vec<ProcessInfo>,
    pub multibar: MultiProgress,
}

impl<'source> TestBed<'source> {
    pub fn new(
        template_output: PathBuf,
        template_includes: Vec<PathBuf>,
        var_names: VarNames,
    ) -> Self {
        let templates = TemplateBuilder::new(template_output, template_includes);
        let progress = MultiProgress::with_draw_target(ProgressDrawTarget::stdout());

        Self {
            templates,
            var_names,
            spawn_limit: None,
            processes: vec![],
            multibar: progress,
        }
    }
}

impl<'source> Executable<Command> for TestBed<'source> {
    fn shutdown(&mut self) {
        for mut value in self.processes.drain(..) {
            value.kill();
        }
    }

    fn finish(&mut self, _: &mut ProgramState, shutdown: &crate::program::Shutdown) {
        let mut kill = false;

        for mut value in self.processes.drain(..) {
            if shutdown.is_shutdown() {
                kill = true;
                break;
            }
            value.wait_or_terminate(None, shutdown);
        }

        if kill {
            <Self as Executable<Command>>::shutdown(self);
        }
    }

    fn execute(
        &mut self,
        command: &Command,
        stack: &mut ProgramState,
        shutdown: &crate::program::Shutdown,
    ) {
        match command {
            Command::LimitSpawn(limit) => self.spawn_limit = Some(*limit),
            Command::Sleep(millis) => {
                let duration = Duration::from_millis(*millis);
                let start = std::time::Instant::now();

                while start.elapsed() < duration {
                    if shutdown.is_shutdown() {
                        break;
                    }
                    std::thread::sleep(SLEEP_TIME);
                }
            }
            Command::Spawn(spawn) => {
                if let Some(limit) = self.spawn_limit {
                    if self.processes.len() >= limit {
                        <Self as Executable<Command>>::finish(self, stack, shutdown);
                    }
                }

                let mut process = spawn.evaluate(stack);
                if let Err(e) = process.run(&self.multibar, 2) {
                    self.multibar
                        .println(&format!("Failed to spawn {}: {e}", process.command))
                        .ok();
                    return;
                }

                self.processes.push(process);
            }
            Command::WaitAll(timeout) => {
                let duration = timeout.map(|value| Duration::from_millis(value));
                for mut value in self.processes.drain(..) {
                    if shutdown.is_shutdown() {
                        break;
                    }
                    value.wait_or_terminate(duration, shutdown);
                }
            }
        }
    }
}

impl<'source> Executable<TemplateCommand> for TestBed<'source> {
    fn shutdown(&mut self) {}

    fn finish(&mut self, _: &mut ProgramState, _: &crate::program::Shutdown) {}

    fn execute(
        &mut self,
        command: &TemplateCommand,
        state: &mut ProgramState,
        _: &crate::program::Shutdown,
    ) {
        match command {
            TemplateCommand::Yield { output, object } => {
                let object = object.evaluate(state);
                self.templates
                    .build(*output, object, state, &self.var_names);
            }
        }
    }
}
