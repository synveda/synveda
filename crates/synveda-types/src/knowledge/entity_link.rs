//! Business links between Knowledge items (ADR-0164 decision 3). A predicate
//! is a compact IRI and states which items it may join; only the IRIs are
//! stored, never vocabulary labels or definitions.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{EntityClass, KnowledgeType};
use crate::{EntityLinkId, Error, KnowledgeItemId, KnowledgeRevisionId, Result, TenantId};
use EntityClass as C;
use LinkEndpoint::{Class, Type};

/// A governed business-link predicate, named by its compact IRI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "&'static str")]
pub enum EntityPredicate {
    /// `org:memberOf`.
    MemberOf,
    /// `org:headOf`.
    HeadOf,
    /// `org:reportsTo`.
    ReportsTo,
    /// `org:subOrganizationOf`.
    SubOrganizationOf,
    /// `schema:worksFor`.
    WorksFor,
    /// `schema:owns`.
    Owns,
    /// `prov:wasAttributedTo`: who made a decision.
    WasAttributedTo,
    /// `dcterms:subject`: what a decision, convention or warning is about.
    Subject,
    /// `synveda:customerOf`, where no standard term fits.
    CustomerOf,
    /// `synveda:dependsOn`, where no standard term fits.
    DependsOn,
    /// `synveda:accountableFor`, where no standard term fits.
    AccountableFor,
}

/// What one end of a predicate admits: an `entity` item of a class, or an
/// item of a Knowledge type that has no class, such as a decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkEndpoint {
    /// An `entity` item declaring this class.
    Class(EntityClass),
    /// An item of this Knowledge type.
    Type(KnowledgeType),
}

/// The facts about one item that a predicate's endpoint rule reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkEndpointItem {
    /// The item's Knowledge type.
    pub knowledge_type: KnowledgeType,
    /// The class its current revision declares, if any.
    pub entity_class: Option<EntityClass>,
}

impl LinkEndpoint {
    fn admits(self, item: LinkEndpointItem) -> bool {
        match self {
            Self::Class(class) => {
                item.knowledge_type == KnowledgeType::Entity && item.entity_class == Some(class)
            }
            Self::Type(knowledge_type) => item.knowledge_type == knowledge_type,
        }
    }
}

// Endpoint sets several predicates share; the others sit beside their rule.
const AGENTS: &[LinkEndpoint] = &[
    Class(C::Person),
    Class(C::Organization),
    Class(C::SoftwareAgent),
];
const PEOPLE: &[LinkEndpoint] = &[Class(C::Person)];
const ORGANISATIONS: &[LinkEndpoint] = &[Class(C::Organization), Class(C::OrganizationalUnit)];
const SYSTEMS: &[LinkEndpoint] = &[
    Class(C::Product),
    Class(C::Service),
    Class(C::SoftwareApplication),
    Class(C::System),
];
const ANY_CLASS: &[LinkEndpoint] = &[
    Class(C::Person),
    Class(C::Organization),
    Class(C::OrganizationalUnit),
    Class(C::Product),
    Class(C::Service),
    Class(C::SoftwareApplication),
    Class(C::SoftwareAgent),
    Class(C::Place),
    Class(C::Event),
    Class(C::Account),
    Class(C::Contract),
    Class(C::System),
];

impl EntityPredicate {
    /// Every predicate in canonical order.
    pub const ALL: &'static [Self] = &[
        Self::MemberOf,
        Self::HeadOf,
        Self::ReportsTo,
        Self::SubOrganizationOf,
        Self::WorksFor,
        Self::Owns,
        Self::WasAttributedTo,
        Self::Subject,
        Self::CustomerOf,
        Self::DependsOn,
        Self::AccountableFor,
    ];

    /// Compact IRI used on the wire and in storage.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MemberOf => "org:memberOf",
            Self::HeadOf => "org:headOf",
            Self::ReportsTo => "org:reportsTo",
            Self::SubOrganizationOf => "org:subOrganizationOf",
            Self::WorksFor => "schema:worksFor",
            Self::Owns => "schema:owns",
            Self::WasAttributedTo => "prov:wasAttributedTo",
            Self::Subject => "dcterms:subject",
            Self::CustomerOf => "synveda:customerOf",
            Self::DependsOn => "synveda:dependsOn",
            Self::AccountableFor => "synveda:accountableFor",
        }
    }

    /// The items a link with this predicate may start from.
    #[must_use]
    pub const fn sources(self) -> &'static [LinkEndpoint] {
        match self {
            Self::MemberOf => AGENTS,
            Self::HeadOf | Self::WorksFor => PEOPLE,
            Self::ReportsTo => &[Class(C::Person), Class(C::SoftwareAgent)],
            Self::SubOrganizationOf => ORGANISATIONS,
            Self::Owns => &[
                Class(C::Person),
                Class(C::Organization),
                Class(C::OrganizationalUnit),
            ],
            Self::WasAttributedTo => &[Type(KnowledgeType::Decision)],
            Self::Subject => &[
                Type(KnowledgeType::Decision),
                Type(KnowledgeType::Convention),
                Type(KnowledgeType::Warning),
            ],
            Self::CustomerOf => &[Class(C::Person), Class(C::Organization), Class(C::Account)],
            Self::DependsOn => SYSTEMS,
            Self::AccountableFor => &[
                Class(C::Person),
                Class(C::Organization),
                Class(C::OrganizationalUnit),
                Class(C::SoftwareAgent),
            ],
        }
    }

    /// The items a link with this predicate may point to.
    #[must_use]
    pub const fn targets(self) -> &'static [LinkEndpoint] {
        match self {
            Self::MemberOf | Self::HeadOf | Self::SubOrganizationOf | Self::WorksFor => {
                ORGANISATIONS
            }
            Self::ReportsTo => PEOPLE,
            Self::Owns => &[
                Class(C::Product),
                Class(C::Service),
                Class(C::SoftwareApplication),
                Class(C::Place),
                Class(C::System),
            ],
            Self::WasAttributedTo => AGENTS,
            Self::Subject => ANY_CLASS,
            Self::CustomerOf => &[
                Class(C::Organization),
                Class(C::OrganizationalUnit),
                Class(C::Product),
                Class(C::Service),
            ],
            Self::DependsOn => SYSTEMS,
            Self::AccountableFor => &[
                Class(C::Product),
                Class(C::Service),
                Class(C::SoftwareApplication),
                Class(C::Account),
                Class(C::Contract),
                Class(C::System),
            ],
        }
    }

    /// Whether this predicate may join `source` to `target`.
    #[must_use]
    pub fn admits(self, source: LinkEndpointItem, target: LinkEndpointItem) -> bool {
        self.sources().iter().any(|end| end.admits(source))
            && self.targets().iter().any(|end| end.admits(target))
    }
}

impl fmt::Display for EntityPredicate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<EntityPredicate> for &'static str {
    fn from(predicate: EntityPredicate) -> Self {
        predicate.as_str()
    }
}

// Only the compact IRI string deserialises, as for `EntityClass`.
impl TryFrom<String> for EntityPredicate {
    type Error = Error;

    fn try_from(value: String) -> Result<Self> {
        value.parse()
    }
}

impl FromStr for EntityPredicate {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|predicate| predicate.as_str() == value)
            .ok_or_else(|| Error::Invalid {
                message: format!("unknown business link predicate: {value:?}"),
            })
    }
}

/// One append-only business link and, when known, its end.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityLink {
    /// Stable link id.
    pub id: EntityLinkId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// Item asserting the link.
    pub source_item_id: KnowledgeItemId,
    /// Item the link points to.
    pub target_item_id: KnowledgeItemId,
    /// Source revision current when the link was asserted.
    pub asserting_revision_id: KnowledgeRevisionId,
    /// Governed predicate.
    pub predicate: EntityPredicate,
    /// Start of the valid-time window.
    pub valid_from: DateTime<Utc>,
    /// End of the valid-time window, exclusive, once an end is recorded.
    pub valid_to: Option<DateTime<Utc>>,
    /// Actor that asserted the link.
    pub created_by: Option<String>,
    /// When the link was recorded.
    pub created_at: DateTime<Utc>,
    /// When the end was recorded.
    pub ended_at: Option<DateTime<Utc>>,
}

impl EntityLink {
    /// The end that is not `item_id`.
    #[must_use]
    pub fn other_end(&self, item_id: KnowledgeItemId) -> KnowledgeItemId {
        if self.source_item_id == item_id {
            self.target_item_id
        } else {
            self.source_item_id
        }
    }

    /// Whether the link holds at `at`: started, and not ended by then.
    #[must_use]
    pub fn is_valid_at(&self, at: DateTime<Utc>) -> bool {
        self.valid_from <= at && self.valid_to.is_none_or(|end| end > at)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const fn entity(class: EntityClass) -> LinkEndpointItem {
        LinkEndpointItem {
            knowledge_type: KnowledgeType::Entity,
            entity_class: Some(class),
        }
    }

    const fn typed(knowledge_type: KnowledgeType) -> LinkEndpointItem {
        LinkEndpointItem {
            knowledge_type,
            entity_class: None,
        }
    }

    #[test]
    fn wire_names_agree_with_serde_and_round_trip() {
        for predicate in EntityPredicate::ALL {
            assert_eq!(
                serde_json::to_value(predicate).unwrap(),
                json!(predicate.as_str())
            );
            assert_eq!(
                serde_json::from_value::<EntityPredicate>(json!(predicate.as_str())).unwrap(),
                *predicate
            );
            assert_eq!(
                predicate.as_str().parse::<EntityPredicate>().unwrap(),
                *predicate
            );
        }
        assert!(serde_json::from_value::<EntityPredicate>(json!({"org:memberOf": null})).is_err());
        let mut sorted = EntityPredicate::ALL.to_vec();
        sorted.sort();
        assert_eq!(sorted, EntityPredicate::ALL, "ALL is in canonical order");
        assert!("org:hasMember".parse::<EntityPredicate>().is_err());
    }

    #[test]
    fn every_predicate_targets_an_entity_class() {
        // An empty class allowlist therefore disables every link.
        for predicate in EntityPredicate::ALL {
            assert!(
                predicate
                    .targets()
                    .iter()
                    .all(|end| matches!(end, LinkEndpoint::Class(_))),
                "{predicate}"
            );
        }
    }

    #[test]
    fn any_class_is_every_declared_class() {
        let declared: Vec<LinkEndpoint> = EntityClass::ALL.iter().copied().map(Class).collect();
        assert_eq!(ANY_CLASS, declared);
    }

    #[test]
    fn a_decision_names_who_decided_and_what_it_is_about() {
        let decision = typed(KnowledgeType::Decision);
        let person = entity(EntityClass::Person);
        let service = entity(EntityClass::Service);
        assert!(EntityPredicate::WasAttributedTo.admits(decision, person));
        assert!(EntityPredicate::Subject.admits(decision, service));
        assert!(EntityPredicate::Subject.admits(typed(KnowledgeType::Warning), service));
        assert!(!EntityPredicate::WasAttributedTo.admits(decision, service));
        assert!(!EntityPredicate::WasAttributedTo.admits(typed(KnowledgeType::Fact), person));
        assert!(!EntityPredicate::Subject.admits(typed(KnowledgeType::Fact), service));
    }

    #[test]
    fn class_endpoints_need_the_declared_class() {
        let person = entity(EntityClass::Person);
        let team = entity(EntityClass::OrganizationalUnit);
        assert!(EntityPredicate::WorksFor.admits(person, team));
        assert!(!EntityPredicate::WorksFor.admits(team, person));
        let misdeclared = LinkEndpointItem {
            knowledge_type: KnowledgeType::Fact,
            entity_class: Some(EntityClass::Person),
        };
        assert!(!EntityPredicate::WorksFor.admits(misdeclared, team));
        let unclassified = typed(KnowledgeType::Entity);
        assert!(!EntityPredicate::WorksFor.admits(unclassified, team));
        assert!(!EntityPredicate::Subject.admits(typed(KnowledgeType::Decision), unclassified));
    }
}
