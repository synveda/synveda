//! The closed cost classes every catalogued application operation declares.

/// What one admitted request may cost the database or an external provider.
///
/// The route catalogue requires one class per operation, so a new route
/// cannot compile without choosing its budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OperationClass {
    /// Bounded single-resource and catalogue reads.
    Read,
    /// Knowledge and audit queries whose cost grows with stored history.
    Search,
    /// Session lifecycle and event, usage and feedback recording.
    SessionIngest,
    /// Retrieval and context planning on the Session hot path.
    ContextCompose,
    /// Governed and administrative state changes.
    Mutation,
    /// Exports, imports, verification, tests and provider-bound queuing.
    Bulk,
}

impl OperationClass {
    /// Every class, in index order.
    pub const ALL: [Self; 6] = [
        Self::Read,
        Self::Search,
        Self::SessionIngest,
        Self::ContextCompose,
        Self::Mutation,
        Self::Bulk,
    ];

    /// Stable configuration key and metric label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Search => "search",
            Self::SessionIngest => "session_ingest",
            Self::ContextCompose => "context_compose",
            Self::Mutation => "mutation",
            Self::Bulk => "bulk",
        }
    }

    pub(crate) const fn index(self) -> usize {
        match self {
            Self::Read => 0,
            Self::Search => 1,
            Self::SessionIngest => 2,
            Self::ContextCompose => 3,
            Self::Mutation => 4,
            Self::Bulk => 5,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indices_are_the_positions_in_all() {
        for (position, class) in OperationClass::ALL.into_iter().enumerate() {
            assert_eq!(class.index(), position, "{}", class.as_str());
        }
    }
}
