//! Granularity adapter keeping replay scheduling independent of attribution units.

use credit_core::{AuthorId, ChangeKind, FileState, LaborDelta, Ledger, TokenState};

use crate::{Error, Granularity, Options};

/// A shared file snapshot contains only the selected attribution representation.
#[derive(Default)]
pub(crate) enum State {
    #[default]
    Empty,
    Line(FileState),
    Token(TokenState),
}

impl State {
    /// Compute a file's conserved ownership in millionths of a line.
    pub(crate) fn ownership(&self) -> Result<LaborDelta, Error> {
        match self {
            Self::Empty => Ok(LaborDelta::new()),
            Self::Line(state) => {
                let mut ledger = Ledger::default();
                ledger.recount([state])?;
                Ok(ledger.ownership)
            }
            Self::Token(state) => Ok(state.ownership()?),
        }
    }

    /// Whether any meaningful content survives, without allocating a ledger.
    pub(crate) fn is_empty(&self) -> bool {
        match self {
            Self::Empty => true,
            Self::Line(state) => state.lines.iter().all(|line| line.text.trim().is_empty()),
            Self::Token(state) => state.tokens().is_empty(),
        }
    }
}

/// Staged changes retain granularity-specific refund and removal metadata.
pub(crate) enum Change {
    Line(credit_core::Change),
    Token(credit_core::TokenChange),
}

impl Change {
    /// Apply the selected engine with the same raw text and normalized language mode.
    pub(crate) fn apply(
        previous: &State,
        text: &str,
        authors: &[(AuthorId, u32)],
        path: &str,
        options: &Options,
    ) -> Result<Self, Error> {
        let mode = crate::normalization(path);
        Ok(match options.granularity {
            Granularity::Line => {
                let empty = FileState::default();
                let previous = if let State::Line(state) = previous {
                    state
                } else {
                    &empty
                };
                Self::Line(credit_core::apply_change(
                    previous,
                    text,
                    authors,
                    mode,
                    options.policy,
                )?)
            }
            Granularity::Token => {
                let empty = TokenState::default();
                let previous = if let State::Token(state) = previous {
                    state
                } else {
                    &empty
                };
                Self::Token(credit_core::apply_token_change(
                    previous, text, authors, mode,
                )?)
            }
        })
    }

    /// Adopt caller-verified exact blob provenance without losing move sources.
    pub(crate) fn adopt(&mut self, parent: &State) {
        match (self, parent) {
            (Self::Line(change), State::Line(parent)) => credit_core::adopt_parent(change, parent),
            (Self::Token(change), State::Token(parent)) => {
                credit_core::adopt_token_parent(change, parent);
            }
            _ => {}
        }
    }

    /// Reconcile matching provenance in recorded Git parent order.
    pub(crate) fn parents<'a>(&mut self, parents: impl Iterator<Item = &'a State>) {
        match self {
            Self::Line(change) => {
                credit_core::reconcile_parents(
                    change,
                    parents.filter_map(|state| {
                        if let State::Line(state) = state {
                            Some(state)
                        } else {
                            None
                        }
                    }),
                );
            }
            Self::Token(change) => {
                credit_core::reconcile_token_parents(
                    change,
                    parents.filter_map(|state| {
                        if let State::Token(state) = state {
                            Some(state)
                        } else {
                            None
                        }
                    }),
                );
            }
        }
    }

    /// Classify canonical changes before structural move reconciliation.
    pub(crate) fn kind(&self) -> ChangeKind {
        match self {
            Self::Line(change) => change.kind,
            Self::Token(change) => change.kind(),
        }
    }

    /// Surface only the line engine's bounded similarity-pairing warning.
    pub(crate) fn pairing_capped(&self) -> bool {
        matches!(self, Self::Line(change) if change.pairing_capped)
    }

    /// Finalize labor after all refunds, and consume the transient change metadata.
    pub(crate) fn finish(self) -> Result<(State, LaborDelta), Error> {
        Ok(match self {
            Self::Line(change) => (State::Line(change.state), change.labor),
            Self::Token(change) => {
                let labor = change.labor()?;
                (State::Token(change.into_state()), labor)
            }
        })
    }
}

/// Partition staged changes by representation without cloning token snapshots.
/// Each analysis uses one granularity; supporting both here keeps that invariant
/// explicit and avoids panics if an empty stage is supplied.
pub(crate) fn reconcile(
    changes: &mut Vec<(String, Change)>,
    parents: Vec<Vec<(String, Change)>>,
    bulk: bool,
) -> bool {
    let mut line = Vec::new();
    let mut token = Vec::new();
    for (path, change) in std::mem::take(changes) {
        match change {
            Change::Line(change) => line.push((path, change)),
            Change::Token(change) => token.push((path, change)),
        }
    }
    let mut line_parents = Vec::new();
    let mut token_parents = Vec::new();
    for parent in parents {
        let mut lines = Vec::new();
        let mut tokens = Vec::new();
        for (path, change) in parent {
            match change {
                Change::Line(change) => lines.push((path, change)),
                Change::Token(change) => tokens.push((path, change)),
            }
        }
        line_parents.push(lines);
        token_parents.push(tokens);
    }
    credit_core::reconcile_merge_moves(&mut line, &line_parents);
    credit_core::reconcile_token_moves(&mut token, &token_parents);
    let (paths, mut tokens): (Vec<_>, Vec<_>) = token.into_iter().unzip();
    let discounted = credit_core::discount_bulk_edits(&mut tokens, bulk);
    changes.extend(
        line.into_iter()
            .map(|(path, change)| (path, Change::Line(change))),
    );
    changes.extend(
        paths
            .into_iter()
            .zip(tokens)
            .map(|(path, change)| (path, Change::Token(change))),
    );
    changes.sort_by(|a, b| a.0.cmp(&b.0));
    discounted
}

/// Sum file ownership with the ledger's checked aggregation, reusing the same
/// primitive for HEAD and directory reports without changing historical labor.
pub(crate) fn ownership<'a>(states: impl Iterator<Item = &'a State>) -> Result<LaborDelta, Error> {
    let mut ledger = Ledger::default();
    for state in states {
        ledger.add_labor(&state.ownership()?)?;
    }
    Ok(ledger.labor)
}
