//! The business-link migration checks exactly the vocabularies the types
//! crate declares, identically in both lineages (ADR-0164).

use synveda_types::knowledge::{EntityPredicate, KnowledgeCommandKind};

const PUBLISHED: &str = include_str!("../migrations/0020_entity_links.sql");
const PORTABLE: &str = include_str!("../portable-migrations/0020_entity_links.sql");

/// The quoted values of the `IN (...)` list that follows `marker`.
fn checked_values<'a>(migration: &'a str, marker: &str) -> Vec<&'a str> {
    let list = &migration[migration.find(marker).expect(marker) + marker.len()..];
    let list = &list[..list.find(')').expect("the list closes")];
    list.split(',')
        .map(|value| value.trim().trim_matches('\''))
        .collect()
}

#[test]
fn both_lineages_check_exactly_the_declared_vocabularies() {
    assert_eq!(PUBLISHED, PORTABLE);
    let predicates: Vec<&str> = EntityPredicate::ALL.iter().map(|p| p.as_str()).collect();
    assert_eq!(checked_values(PUBLISHED, "predicate IN ("), predicates);
    let mut checked = checked_values(PUBLISHED, "(command_kind IN (");
    let mut declared: Vec<&str> = KnowledgeCommandKind::ALL
        .iter()
        .map(|k| k.as_str())
        .collect();
    checked.sort_unstable();
    declared.sort_unstable();
    assert_eq!(checked, declared);
}
