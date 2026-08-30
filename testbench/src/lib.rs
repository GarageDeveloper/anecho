//! A/B bench plumbing (phase 0): a client for REW's local HTTP API and a thin wrapper over
//! the Anecho client, plus trivial comparisons. Numerical comparisons (FR, IR, THD, RT60
//! with documented tolerances) arrive with phase 2.

pub mod anecho;
pub mod rew;

/// Outcome of one comparison line.
#[derive(Debug, Clone, PartialEq)]
pub struct Check {
    pub name: String,
    pub rew: String,
    pub anecho: String,
    pub ok: bool,
}

impl std::fmt::Display for Check {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} {:<24} REW: {:<40} Anecho: {}",
            if self.ok { "ok " } else { "!! " },
            self.name,
            self.rew,
            self.anecho
        )
    }
}
