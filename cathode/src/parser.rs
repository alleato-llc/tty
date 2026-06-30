use crate::screen::TerminalScreen;
use vte::Parser;

pub struct TermParser {
    parser: Parser,
}

impl TermParser {
    pub fn new() -> Self {
        Self {
            parser: Parser::new(),
        }
    }

    pub fn process(&mut self, data: &[u8], screen: &mut TerminalScreen) {
        self.parser.advance(screen, data);
    }
}

impl Default for TermParser {
    fn default() -> Self {
        Self::new()
    }
}
