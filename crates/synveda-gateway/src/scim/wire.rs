//! The SCIM 2.0 wire format (AUTH-4, ADR-0059 decision 1): RFC 7643
//! resources and RFC 7644 messages, as this server speaks them.
//!
//! Everything here is somebody else's shape. The product's own error
//! envelope, its `snake_case` bodies and its route conventions stop at the
//! `/scim/v2` boundary, because the audience for these bytes is a
//! provisioning agent that will report whatever it cannot parse to an
//! administrator as a failure of ours. It is ADR-0051's inversion — a
//! skill's bytes are read by a loader we do not ship — arriving on the
//! transport side.
//!
//! **Unknown attributes are ignored rather than refused.** RFC 7644 §3.3
//! permits either, and Entra sends the enterprise-user extension and a
//! handful of attributes on every request whether or not a server asked for
//! them. Refusing would mean refusing every real client; what keeps that
//! honest is that `/Schemas` publishes exactly the attributes this server
//! stores, so an attribute it drops is one it never claimed to keep
//! (migration 0036's header).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use synveda_types::{DirectoryGroup, DirectoryUser};

/// The core `User` schema URN.
pub const USER_SCHEMA: &str = "urn:ietf:params:scim:schemas:core:2.0:User";
/// The core `Group` schema URN.
pub const GROUP_SCHEMA: &str = "urn:ietf:params:scim:schemas:core:2.0:Group";
/// The `ListResponse` message URN.
pub const LIST_RESPONSE_SCHEMA: &str = "urn:ietf:params:scim:api:messages:2.0:ListResponse";
/// The `PatchOp` message URN.
pub const PATCH_OP_SCHEMA: &str = "urn:ietf:params:scim:api:messages:2.0:PatchOp";
/// The `Error` message URN.
pub const ERROR_SCHEMA: &str = "urn:ietf:params:scim:api:messages:2.0:Error";
/// The `ServiceProviderConfig` schema URN.
pub const SERVICE_PROVIDER_CONFIG_SCHEMA: &str =
    "urn:ietf:params:scim:schemas:core:2.0:ServiceProviderConfig";
/// The `ResourceType` schema URN.
pub const RESOURCE_TYPE_SCHEMA: &str = "urn:ietf:params:scim:schemas:core:2.0:ResourceType";
/// The `Schema` schema URN.
pub const SCHEMA_SCHEMA: &str = "urn:ietf:params:scim:schemas:core:2.0:Schema";

/// The media type RFC 7644 §3.1 assigns to every SCIM body.
pub const SCIM_CONTENT_TYPE: &str = "application/scim+json";

/// A resource's `meta` complex attribute.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meta {
    /// `User` or `Group`.
    #[serde(rename = "resourceType")]
    pub resource_type: String,
    /// When the resource was created.
    pub created: DateTime<Utc>,
    /// When it last changed.
    #[serde(rename = "lastModified")]
    pub last_modified: DateTime<Utc>,
    /// Its absolute (or prefix-relative) URL.
    pub location: String,
    /// The ETag, weak-form per RFC 7644 §3.14.
    pub version: String,
}

/// `name` — the three sub-attributes this server stores.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Name {
    /// `name.givenName`.
    #[serde(rename = "givenName", skip_serializing_if = "Option::is_none")]
    pub given_name: Option<String>,
    /// `name.familyName`.
    #[serde(rename = "familyName", skip_serializing_if = "Option::is_none")]
    pub family_name: Option<String>,
    /// `name.formatted`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub formatted: Option<String>,
}

/// One entry of a multi-valued attribute (`emails`, `members`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MultiValue {
    /// The entry's value — an address, or a member id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// `type`, e.g. `work`.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub value_type: Option<String>,
    /// Whether this is the primary entry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary: Option<bool>,
    /// A human label for the entry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    /// `$ref` on a group member — emitted, never read.
    #[serde(rename = "$ref", skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
}

/// A `User` resource, in and out.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserResource {
    /// The resource's schema URNs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub schemas: Vec<String>,
    /// The resource id, on the way out.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The directory's own anchor.
    #[serde(rename = "externalId", skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
    /// `userName`.
    #[serde(rename = "userName", default)]
    pub user_name: Option<String>,
    /// Absent means `true`: RFC 7643 §4.1.1 defaults it, and Okta omits it
    /// on create.
    #[serde(default)]
    pub active: Option<bool>,
    /// `displayName`.
    #[serde(rename = "displayName", skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// `name`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<Name>,
    /// `emails`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub emails: Vec<MultiValue>,
    /// `meta`, on the way out.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
}

impl UserResource {
    /// The `work` email, or the primary one, or the first — the order a
    /// directory's own precedence rules imply, resolved in one place so no
    /// route invents a second.
    #[must_use]
    pub fn work_email(&self) -> Option<String> {
        let by_type = self
            .emails
            .iter()
            .find(|email| email.value_type.as_deref() == Some("work"));
        let primary = self.emails.iter().find(|email| email.primary == Some(true));
        by_type
            .or(primary)
            .or_else(|| self.emails.first())
            .and_then(|email| email.value.clone())
    }

    /// Renders a stored mirror row as the resource a client reads back.
    #[must_use]
    pub fn of(user: &DirectoryUser, base: &str) -> UserResource {
        let emails = user
            .work_email
            .as_ref()
            .map(|email| {
                vec![MultiValue {
                    value: Some(email.clone()),
                    value_type: Some("work".to_owned()),
                    primary: Some(true),
                    ..MultiValue::default()
                }]
            })
            .unwrap_or_default();
        UserResource {
            schemas: vec![USER_SCHEMA.to_owned()],
            id: Some(user.id.to_string()),
            external_id: user.external_id.clone(),
            user_name: Some(user.user_name.clone()),
            active: Some(user.active),
            display_name: user.display_name.clone(),
            name: Some(Name {
                given_name: user.given_name.clone(),
                family_name: user.family_name.clone(),
                formatted: user.display_name.clone(),
            }),
            emails,
            meta: Some(Meta {
                resource_type: "User".to_owned(),
                created: user.created_at,
                last_modified: user.updated_at,
                location: format!("{base}/Users/{}", user.id),
                version: etag(user.version),
            }),
        }
    }
}

/// A `Group` resource, in and out.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GroupResource {
    /// The resource's schema URNs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub schemas: Vec<String>,
    /// The resource id, on the way out.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The directory's own anchor.
    #[serde(rename = "externalId", skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
    /// `displayName`.
    #[serde(rename = "displayName", default)]
    pub display_name: Option<String>,
    /// `members`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<MultiValue>,
    /// `meta`, on the way out.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
}

impl GroupResource {
    /// Renders a stored group and its membership as the resource a client
    /// reads back.
    #[must_use]
    pub fn of(group: &DirectoryGroup, members: &[DirectoryUser], base: &str) -> GroupResource {
        GroupResource {
            schemas: vec![GROUP_SCHEMA.to_owned()],
            id: Some(group.id.to_string()),
            external_id: group.external_id.clone(),
            display_name: Some(group.display_name.clone()),
            members: members
                .iter()
                .map(|member| MultiValue {
                    value: Some(member.id.to_string()),
                    display: Some(member.user_name.clone()),
                    reference: Some(format!("{base}/Users/{}", member.id)),
                    ..MultiValue::default()
                })
                .collect(),
            meta: Some(Meta {
                resource_type: "Group".to_owned(),
                created: group.created_at,
                last_modified: group.updated_at,
                location: format!("{base}/Groups/{}", group.id),
                version: etag(group.version),
            }),
        }
    }
}

/// The weak ETag form RFC 7644 §3.14 specifies.
#[must_use]
pub fn etag(version: i64) -> String {
    format!("W/\"{version}\"")
}

/// A `ListResponse` — the envelope every query answers in, including the
/// empty one.
#[derive(Debug, Clone, Serialize)]
pub struct ListResponse<T> {
    /// The resource's schema URNs.
    pub schemas: Vec<String>,
    /// The whole matching count, not the page's.
    #[serde(rename = "totalResults")]
    pub total_results: i64,
    /// The 1-based index this page starts at.
    #[serde(rename = "startIndex")]
    pub start_index: i64,
    /// How many resources this page carries.
    #[serde(rename = "itemsPerPage")]
    pub items_per_page: i64,
    /// The page itself.
    #[serde(rename = "Resources")]
    pub resources: Vec<T>,
}

impl<T> ListResponse<T> {
    /// One page of results. `total_results` is the whole count rather than
    /// the page's, which RFC 7644 §3.4.2 requires and which a client uses
    /// to decide whether to ask again.
    pub fn new(resources: Vec<T>, total_results: i64, start_index: i64) -> Self {
        ListResponse {
            schemas: vec![LIST_RESPONSE_SCHEMA.to_owned()],
            total_results,
            start_index,
            items_per_page: i64::try_from(resources.len()).unwrap_or(i64::MAX),
            resources,
        }
    }
}

/// One operation of a `PatchOp` request.
#[derive(Debug, Clone, Deserialize)]
pub struct PatchOperation {
    /// `add`, `remove` or `replace`. Compared case-insensitively: Entra
    /// capitalises them and RFC 7644 §3.5.2 does not.
    pub op: String,
    /// The attribute path, when the client sends one.
    #[serde(default)]
    pub path: Option<String>,
    /// The operand.
    #[serde(default)]
    pub value: Option<Value>,
}

/// A `PatchOp` request body.
#[derive(Debug, Clone, Deserialize)]
pub struct PatchRequest {
    /// The resource's schema URNs.
    #[serde(default)]
    pub schemas: Vec<String>,
    /// Capital `O` — RFC 7644 §3.5.2's own spelling, and one of the
    /// details a hand-rolled client gets wrong.
    #[serde(rename = "Operations", default)]
    pub operations: Vec<PatchOperation>,
}

/// `/ServiceProviderConfig` — what this server actually implements.
///
/// Generated from the same constants the routes enforce rather than
/// hand-written, so the advertisement cannot drift from the behaviour. The
/// conformance suite asserts exactly that pairing.
#[must_use]
pub fn service_provider_config(base: &str, max_results: i64) -> Value {
    json!({
        "schemas": [SERVICE_PROVIDER_CONFIG_SCHEMA],
        "documentationUri": "https://synveda.dev/docs/scim",
        "patch": {"supported": true},
        // No bulk: neither AC client requires it, and a bulk endpoint is a
        // second transaction model over the same reconciler.
        "bulk": {"supported": false, "maxOperations": 0, "maxPayloadSize": 0},
        "filter": {"supported": true, "maxResults": max_results},
        // The IdP owns credentials; this server has no password to change
        // and no interest in acquiring one.
        "changePassword": {"supported": false},
        "sort": {"supported": false},
        // Advertised because `meta.version` is real: every write bumps a
        // counter, and a client may use it to skip work.
        "etag": {"supported": true},
        "authenticationSchemes": [{
            "type": "oauthbearertoken",
            "name": "OAuth Bearer Token",
            "description": "A provisioning credential issued by `synveda scim token issue`.",
            "primary": true
        }],
        "meta": {
            "resourceType": "ServiceProviderConfig",
            "location": format!("{base}/ServiceProviderConfig")
        }
    })
}

/// `/ResourceTypes` — the two resources this server serves.
#[must_use]
pub fn resource_types(base: &str) -> Vec<Value> {
    vec![
        json!({
            "schemas": [RESOURCE_TYPE_SCHEMA],
            "id": "User",
            "name": "User",
            "endpoint": "/Users",
            "description": "SCIM 2.0 User",
            "schema": USER_SCHEMA,
            "meta": {"resourceType": "ResourceType", "location": format!("{base}/ResourceTypes/User")}
        }),
        json!({
            "schemas": [RESOURCE_TYPE_SCHEMA],
            "id": "Group",
            "name": "Group",
            "endpoint": "/Groups",
            "description": "SCIM 2.0 Group",
            "schema": GROUP_SCHEMA,
            "meta": {"resourceType": "ResourceType", "location": format!("{base}/ResourceTypes/Group")}
        }),
    ]
}

/// `/Schemas` — the attributes this server stores, and no others.
///
/// This is the endpoint that makes "unknown attributes are ignored" an
/// honest position rather than a shrug: what is not here was never
/// promised.
#[must_use]
pub fn schemas(base: &str) -> Vec<Value> {
    let attribute = |name: &str, kind: &str, multi: bool, sub: Value| {
        json!({
            "name": name,
            "type": kind,
            "multiValued": multi,
            "required": false,
            "caseExact": false,
            "mutability": "readWrite",
            "returned": "default",
            "uniqueness": "none",
            "subAttributes": sub,
        })
    };
    vec![
        json!({
            "schemas": [SCHEMA_SCHEMA],
            "id": USER_SCHEMA,
            "name": "User",
            "description": "SCIM 2.0 User, as stored by Synveda",
            "attributes": [
                json!({
                    "name": "userName", "type": "string", "multiValued": false,
                    "required": true, "caseExact": false, "mutability": "readWrite",
                    "returned": "default", "uniqueness": "server"
                }),
                attribute("externalId", "string", false, Value::Null),
                attribute("active", "boolean", false, Value::Null),
                attribute("displayName", "string", false, Value::Null),
                attribute("name", "complex", false, json!([
                    {"name": "givenName", "type": "string", "multiValued": false},
                    {"name": "familyName", "type": "string", "multiValued": false},
                    {"name": "formatted", "type": "string", "multiValued": false}
                ])),
                attribute("emails", "complex", true, json!([
                    {"name": "value", "type": "string", "multiValued": false},
                    {"name": "type", "type": "string", "multiValued": false},
                    {"name": "primary", "type": "boolean", "multiValued": false}
                ])),
            ],
            "meta": {"resourceType": "Schema", "location": format!("{base}/Schemas/{USER_SCHEMA}")}
        }),
        json!({
            "schemas": [SCHEMA_SCHEMA],
            "id": GROUP_SCHEMA,
            "name": "Group",
            "description": "SCIM 2.0 Group, as stored by Synveda",
            "attributes": [
                json!({
                    "name": "displayName", "type": "string", "multiValued": false,
                    "required": true, "caseExact": false, "mutability": "readWrite",
                    "returned": "default", "uniqueness": "server"
                }),
                attribute("externalId", "string", false, Value::Null),
                attribute("members", "complex", true, json!([
                    {"name": "value", "type": "string", "multiValued": false},
                    {"name": "display", "type": "string", "multiValued": false}
                ])),
            ],
            "meta": {"resourceType": "Schema", "location": format!("{base}/Schemas/{GROUP_SCHEMA}")}
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_work_email_wins_over_primary_and_first() {
        let user = UserResource {
            emails: vec![
                MultiValue {
                    value: Some("home@example.com".to_owned()),
                    value_type: Some("home".to_owned()),
                    primary: Some(true),
                    ..MultiValue::default()
                },
                MultiValue {
                    value: Some("work@example.com".to_owned()),
                    value_type: Some("work".to_owned()),
                    ..MultiValue::default()
                },
            ],
            ..UserResource::default()
        };
        assert_eq!(user.work_email().as_deref(), Some("work@example.com"));
    }

    #[test]
    fn a_primary_email_is_taken_when_no_work_one_is_typed() {
        let user = UserResource {
            emails: vec![
                MultiValue {
                    value: Some("other@example.com".to_owned()),
                    ..MultiValue::default()
                },
                MultiValue {
                    value: Some("primary@example.com".to_owned()),
                    primary: Some(true),
                    ..MultiValue::default()
                },
            ],
            ..UserResource::default()
        };
        assert_eq!(user.work_email().as_deref(), Some("primary@example.com"));
    }

    #[test]
    fn unknown_attributes_are_ignored_rather_than_refused() {
        // Entra sends the enterprise-user extension on every request. A
        // server that refused it would refuse every real client.
        let body = json!({
            "schemas": [USER_SCHEMA, "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User"],
            "userName": "ada@example.com",
            "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User": {
                "department": "Engineering", "employeeNumber": "42"
            },
            "phoneNumbers": [{"value": "+1-555-0100", "type": "work"}],
            "title": "Engineer"
        });
        let user: UserResource = serde_json::from_value(body).expect("parse");
        assert_eq!(user.user_name.as_deref(), Some("ada@example.com"));
    }

    #[test]
    fn a_patch_body_parses_the_capital_o_spelling() {
        let body = json!({
            "schemas": [PATCH_OP_SCHEMA],
            "Operations": [{"op": "Replace", "path": "active", "value": false}]
        });
        let patch: PatchRequest = serde_json::from_value(body).expect("parse");
        assert_eq!(patch.operations.len(), 1);
        assert_eq!(patch.operations[0].op, "Replace");
    }

    #[test]
    fn an_absent_active_is_absent_rather_than_false() {
        // Okta omits `active` on create, and a server that read the
        // absence as `false` would seal every person it provisioned.
        let user: UserResource =
            serde_json::from_value(json!({"userName": "ada@example.com"})).expect("parse");
        assert_eq!(user.active, None);
    }

    #[test]
    fn the_advertised_config_names_only_what_is_implemented() {
        let config = service_provider_config("https://example.test/scim/v2", 200);
        assert_eq!(config["patch"]["supported"], json!(true));
        assert_eq!(config["bulk"]["supported"], json!(false));
        assert_eq!(config["sort"]["supported"], json!(false));
        assert_eq!(config["filter"]["maxResults"], json!(200));
    }
}
