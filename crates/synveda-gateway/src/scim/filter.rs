//! The SCIM filter subset (AUTH-4, ADR-0059 decision 15).
//!
//! RFC 7644 §3.4.2.2 defines a filter language with grouping, negation,
//! nine comparison operators and complex-attribute traversal. This server
//! implements the equality filters its two AC clients actually send, and
//! answers `501` with `scimType: "invalidFilter"` for everything else —
//! which that same section provides for in as many words: *"If a service
//! provider does not support filtering... the service provider SHOULD
//! return HTTP 501."*
//!
//! The bound is not laziness, and it is worth being precise about why. A
//! filter here compiles to one of a fixed set of compile-time-checked SQL
//! statements. A parser that accepted the whole language would have to
//! either build SQL from strings or reject most of what it parsed at
//! evaluation time — a rejection wearing a conformance badge. Refusing at
//! the door, with the status the RFC names, is the honest version.
//!
//! What is accepted, on both resources: `eq` against one attribute, with a
//! double-quoted value.
//!
//! ```text
//! userName eq "ada@example.com"      (Entra, Okta)
//! externalId eq "9f2c…"              (Entra)
//! id eq "0f6f…"                      (both, on re-read)
//! displayName eq "synveda-eng-core"  (Okta, on Groups)
//! ```

use std::fmt;

/// A parsed filter: one attribute compared for equality with one value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EqFilter {
    /// The attribute name, lower-cased — SCIM attribute names are
    /// case-insensitive (RFC 7643 §2.1) and Entra sends `userName` where
    /// Okta sends `username`.
    pub attribute: String,
    /// The compared value, unescaped.
    pub value: String,
}

/// Why a filter was refused, mapped by the caller onto the `scimType` the
/// RFC assigns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterError {
    /// Syntactically not a filter this server can read at all → 400
    /// `invalidFilter`.
    Malformed(String),
    /// A well-formed filter outside the supported subset → 501
    /// `invalidFilter`.
    Unsupported(String),
}

impl fmt::Display for FilterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FilterError::Malformed(message) | FilterError::Unsupported(message) => {
                f.write_str(message)
            }
        }
    }
}

/// The attributes `Users` may be filtered on.
pub const USER_FILTERABLE: &[&str] = &["username", "externalid", "id"];
/// The attributes `Groups` may be filtered on.
pub const GROUP_FILTERABLE: &[&str] = &["displayname", "externalid", "id"];

/// Parses one `attribute eq "value"` filter, refusing everything else.
///
/// # Errors
///
/// [`FilterError::Malformed`] when the text is not an equality filter at
/// all; [`FilterError::Unsupported`] when it is a filter this server does
/// not implement — including a conjunction, which is well-formed SCIM and
/// simply not something either AC client sends.
pub fn parse(filter: &str, filterable: &[&str]) -> Result<EqFilter, FilterError> {
    let trimmed = filter.trim();
    if trimmed.is_empty() {
        return Err(FilterError::Malformed("empty filter".to_owned()));
    }
    // Grouping, negation and conjunction are all well-formed SCIM this
    // server does not implement. Named individually rather than caught by
    // a generic "did not parse", so an operator reading the message knows
    // whether they wrote something wrong or something unsupported.
    for (needle, what) in [
        (" and ", "conjunction (`and`)"),
        (" or ", "disjunction (`or`)"),
        ("not ", "negation (`not`)"),
        ("(", "grouping"),
        ("[", "complex-attribute filtering"),
    ] {
        if trimmed.to_ascii_lowercase().contains(needle) {
            return Err(FilterError::Unsupported(format!(
                "{what} is not supported; this server filters on a single \
                 `attribute eq \"value\"` comparison"
            )));
        }
    }

    let mut parts = trimmed.splitn(3, char::is_whitespace);
    let attribute = parts
        .next()
        .filter(|part| !part.is_empty())
        .ok_or_else(|| FilterError::Malformed("filter has no attribute".to_owned()))?;
    let operator = parts
        .next()
        .ok_or_else(|| FilterError::Malformed("filter has no operator".to_owned()))?;
    let value = parts
        .next()
        .ok_or_else(|| FilterError::Malformed("filter has no value".to_owned()))?
        .trim();

    if !operator.eq_ignore_ascii_case("eq") {
        // `pr`, `co`, `sw`, `gt`… are all real SCIM and none of them are
        // expressible as one of this server's prepared statements.
        return Err(FilterError::Unsupported(format!(
            "operator `{operator}` is not supported; this server supports `eq`"
        )));
    }

    let value = value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .ok_or_else(|| {
            FilterError::Malformed("filter value must be a double-quoted string".to_owned())
        })?;
    // RFC 7644 filter values are JSON strings, so `\"` and `\\` are the
    // escapes that can appear inside one.
    let value = value.replace("\\\"", "\"").replace("\\\\", "\\");

    let attribute = attribute.to_ascii_lowercase();
    if !filterable.contains(&attribute.as_str()) {
        return Err(FilterError::Unsupported(format!(
            "attribute `{attribute}` is not filterable here; this server \
             filters on {}",
            filterable.join(", ")
        )));
    }
    Ok(EqFilter { attribute, value })
}

/// Extracts the member id from the `members[value eq "…"]` path Entra
/// sends on a `remove` operation — the one place complex-attribute
/// filtering appears in a request this server must honour, and the reason
/// it is handled here rather than refused with the rest of `[`.
#[must_use]
pub fn member_value_path(path: &str) -> Option<String> {
    let trimmed = path.trim();
    let rest = trimmed.strip_prefix("members")?.trim_start();
    let inner = rest.strip_prefix('[')?.strip_suffix(']')?;
    let filter = parse(inner, &["value"]).ok()?;
    Some(filter.value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_filters_both_clients_send_parse() {
        let entra = parse("userName eq \"ada@example.com\"", USER_FILTERABLE).expect("entra");
        assert_eq!(entra.attribute, "username");
        assert_eq!(entra.value, "ada@example.com");

        let okta = parse("externalId eq \"9f2c\"", USER_FILTERABLE).expect("okta");
        assert_eq!(okta.attribute, "externalid");
        assert_eq!(okta.value, "9f2c");
    }

    #[test]
    fn attribute_names_are_case_insensitive() {
        // RFC 7643 §2.1. Entra sends `userName`, hand-rolled clients send
        // `username`, and both mean the same attribute.
        for text in [
            "userName eq \"a\"",
            "username eq \"a\"",
            "USERNAME eq \"a\"",
        ] {
            assert_eq!(
                parse(text, USER_FILTERABLE).expect(text).attribute,
                "username"
            );
        }
    }

    #[test]
    fn well_formed_filters_outside_the_subset_are_unsupported_not_malformed() {
        // The distinction is the whole point of the two variants: one is
        // the caller's mistake (400), the other is this server's boundary
        // (501), and conflating them would tell an administrator to go and
        // fix a correct filter.
        for text in [
            "userName eq \"a\" and active eq true",
            "userName sw \"a\"",
            "not (userName eq \"a\")",
            "emails[type eq \"work\"].value eq \"a\"",
        ] {
            assert!(
                matches!(
                    parse(text, USER_FILTERABLE),
                    Err(FilterError::Unsupported(_))
                ),
                "{text} must be unsupported rather than malformed"
            );
        }
    }

    #[test]
    fn a_filter_on_an_unstored_attribute_is_unsupported() {
        // `title` is a real SCIM attribute this server does not store, so
        // filtering on it can only ever answer wrongly.
        assert!(matches!(
            parse("title eq \"Engineer\"", USER_FILTERABLE),
            Err(FilterError::Unsupported(_))
        ));
        // ...and `displayName` is filterable on Groups but not on Users,
        // which is why the filterable set is a parameter.
        assert!(matches!(
            parse("displayName eq \"x\"", USER_FILTERABLE),
            Err(FilterError::Unsupported(_))
        ));
        assert!(parse("displayName eq \"x\"", GROUP_FILTERABLE).is_ok());
    }

    #[test]
    fn malformed_filters_are_refused_as_malformed() {
        for text in ["", "userName", "userName eq", "userName eq ada"] {
            assert!(
                matches!(parse(text, USER_FILTERABLE), Err(FilterError::Malformed(_))),
                "{text:?} must be malformed"
            );
        }
    }

    #[test]
    fn an_escaped_quote_survives_the_value() {
        let filter = parse(r#"userName eq "a\"b""#, USER_FILTERABLE).expect("parse");
        assert_eq!(filter.value, "a\"b");
    }

    #[test]
    fn the_member_removal_path_yields_its_id() {
        assert_eq!(
            member_value_path("members[value eq \"0f6f-1\"]").as_deref(),
            Some("0f6f-1")
        );
        assert_eq!(member_value_path("members").as_deref(), None);
        assert_eq!(member_value_path("displayName").as_deref(), None);
    }
}
