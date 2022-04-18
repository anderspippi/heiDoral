use crate::language::error::msg::{SemError, SemErrorAndPos};
use crate::language::sem_analysis::{SemAnalysis, SourceFileId};

use dora_parser::lexer::position::Position;

pub struct Diagnostic {
    errors: Vec<SemErrorAndPos>,
}

impl Diagnostic {
    pub fn new() -> Diagnostic {
        Diagnostic { errors: Vec::new() }
    }

    pub fn errors(&self) -> &[SemErrorAndPos] {
        &self.errors
    }

    pub fn report(&mut self, file: SourceFileId, pos: Position, msg: SemError) {
        self.errors.push(SemErrorAndPos::new(file, pos, msg));
    }

    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    pub fn dump(&self, sa: &SemAnalysis) {
        for err in &self.errors {
            eprintln!("{}", &err.message(sa));
        }
    }
}
