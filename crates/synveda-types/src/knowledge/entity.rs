//! Optional entity class declarations in immutable metadata (ADR-0164).
//! Only compact IRIs are stored; no vocabulary labels or definitions are
//! copied.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use super::KnowledgeRevisionContent;
use crate::{Error, Result};

const KEY: &str = "synveda_entity";

/// A class an `entity` item may declare, named by its compact IRI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum EntityClass {
    /// `schema:Person`.
    #[serde(rename = "schema:Person")]
    Person,
    /// `schema:Organization`.
    #[serde(rename = "schema:Organization")]
    Organization,
    /// `org:OrganizationalUnit`.
    #[serde(rename = "org:OrganizationalUnit")]
    OrganizationalUnit,
    /// `schema:Product`.
    #[serde(rename = "schema:Product")]
    Product,
    /// `schema:Service`.
    #[serde(rename = "schema:Service")]
    Service,
    /// `schema:SoftwareApplication`.
    #[serde(rename = "schema:SoftwareApplication")]
    SoftwareApplication,
    /// `prov:SoftwareAgent`.
    #[serde(rename = "prov:SoftwareAgent")]
    SoftwareAgent,
    /// `schema:Place`.
    #[serde(rename = "schema:Place")]
    Place,
    /// `schema:Event`.
    #[serde(rename = "schema:Event")]
    Event,
    /// `synveda:Account`, where no standard term fits.
    #[serde(rename = "synveda:Account")]
    Account,
    /// `synveda:Contract`, where no standard term fits.
    #[serde(rename = "synveda:Contract")]
    Contract,
    /// `synveda:System`, where no standard term fits.
    #[serde(rename = "synveda:System")]
    System,
}

impl EntityClass {
    /// Every class in canonical order.
    pub const ALL: &'static [Self] = &[
        Self::Person,
        Self::Organization,
        Self::OrganizationalUnit,
        Self::Product,
        Self::Service,
        Self::SoftwareApplication,
        Self::SoftwareAgent,
        Self::Place,
        Self::Event,
        Self::Account,
        Self::Contract,
        Self::System,
    ];

    /// Compact IRI used on the wire and in storage.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Person => "schema:Person",
            Self::Organization => "schema:Organization",
            Self::OrganizationalUnit => "org:OrganizationalUnit",
            Self::Product => "schema:Product",
            Self::Service => "schema:Service",
            Self::SoftwareApplication => "schema:SoftwareApplication",
            Self::SoftwareAgent => "prov:SoftwareAgent",
            Self::Place => "schema:Place",
            Self::Event => "schema:Event",
            Self::Account => "synveda:Account",
            Self::Contract => "synveda:Contract",
            Self::System => "synveda:System",
        }
    }
}

impl fmt::Display for EntityClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for EntityClass {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|class| class.as_str() == value)
            .ok_or_else(|| Error::Invalid {
                message: format!("unknown entity class: {value:?}"),
            })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Projection {
    schema_version: u8,
    class: EntityClass,
}

pub(super) fn declared_class(content: &KnowledgeRevisionContent) -> Result<Option<EntityClass>> {
    let Some(value) = content.metadata.get(KEY) else {
        return Ok(None);
    };
    match Projection::deserialize(value) {
        Ok(Projection {
            schema_version: 1,
            class,
        }) => Ok(Some(class)),
        _ => Err(Error::Invalid {
            message: format!(
                "{KEY} must be {{\"schema_version\": 1, \"class\": <entity class compact IRI>}}"
            ),
        }),
    }
}

pub(super) fn validate(content: &KnowledgeRevisionContent) -> Result<()> {
    declared_class(content).map(|_| ())
}

#[cfg(test)]
mod tests {
    use chrono::DateTime;
    use serde_json::json;

    use super::*;
    use crate::Sensitivity;

    fn content(metadata: serde_json::Value) -> KnowledgeRevisionContent {
        KnowledgeRevisionContent {
            title: "Billing service".to_owned(),
            body_markdown: "Charges customers each month.".to_owned(),
            summary: "Charges customers each month.".to_owned(),
            tags: Vec::new(),
            sensitivity: Sensitivity::Internal,
            confidence_permille: 900,
            valid_from: DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
            valid_to: None,
            stale_after: None,
            verification_metadata: json!({}),
            metadata,
        }
    }

    #[test]
    fn wire_names_agree_with_serde_and_round_trip() {
        for class in EntityClass::ALL {
            assert_eq!(serde_json::to_value(class).unwrap(), json!(class.as_str()));
            assert_eq!(class.as_str().parse::<EntityClass>().unwrap(), *class);
        }
        let mut sorted = EntityClass::ALL.to_vec();
        sorted.sort();
        assert_eq!(sorted, EntityClass::ALL, "ALL is in canonical order");
        assert!("schema:Thing".parse::<EntityClass>().is_err());
    }

    #[test]
    fn absent_declaration_is_no_class() {
        assert_eq!(declared_class(&content(json!({}))).unwrap(), None);
    }

    #[test]
    fn a_version_one_declaration_names_its_class() {
        let declared = content(json!({KEY: {"schema_version": 1, "class": "schema:Service"}}));
        assert_eq!(
            declared_class(&declared).unwrap(),
            Some(EntityClass::Service)
        );
    }

    #[test]
    fn malformed_declarations_are_refused() {
        for value in [
            json!("schema:Service"),
            json!({"schema_version": 2, "class": "schema:Service"}),
            json!({"schema_version": 1, "class": "schema:Thing"}),
            json!({"schema_version": 1}),
            json!({"schema_version": 1, "class": "schema:Service", "aliases": []}),
        ] {
            assert!(validate(&content(json!({KEY: value}))).is_err(), "{value}");
        }
    }
}
