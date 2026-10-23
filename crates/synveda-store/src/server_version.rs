//! The finite set of PostgreSQL server majors this build admits
//! ADR-0152).
//!
//! Business SQL is common to every admitted major. Only catalogue-shaped
//! authority proofs differ, and they select their static SQL from this type
//! before PostgreSQL prepares it.

use synveda_types::{Error, Result};

/// One admitted PostgreSQL server major.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PostgresMajor {
    /// PostgreSQL 14.
    V14,
    /// PostgreSQL 15.
    V15,
    /// PostgreSQL 16.
    V16,
    /// PostgreSQL 17.
    V17,
    /// PostgreSQL 18.
    V18,
}

/// How a server records role membership authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MembershipModel {
    /// PostgreSQL 16+ records INHERIT and SET per membership and grantor.
    GrantOptions,
    /// PostgreSQL 14 and 15 derive inheritance from the member's INHERIT
    /// attribute and let every member SET ROLE.
    RoleInheritance,
}

impl PostgresMajor {
    /// Every admitted major, oldest first.
    pub const SUPPORTED: [Self; 5] = [Self::V14, Self::V15, Self::V16, Self::V17, Self::V18];

    /// Selects the admitted major from `server_version_num`, refusing every
    /// other server before version-sensitive SQL runs.
    pub fn from_server_version_num(value: i32) -> Result<Self> {
        match value / 10_000 {
            14 => Ok(Self::V14),
            15 => Ok(Self::V15),
            16 => Ok(Self::V16),
            17 => Ok(Self::V17),
            18 => Ok(Self::V18),
            found => Err(Error::Invalid {
                message: format!(
                    "PostgreSQL server major {found} is not supported: Synveda admits PostgreSQL 14, 15, 16, 17 or 18; \
                     upgrade or select a supported server before installation (nothing was changed); \
                     see docs/DEPLOYMENT_CONTRACT.md#postgresql-compatibility"
                ),
            }),
        }
    }

    /// The major version number.
    #[must_use]
    pub fn number(self) -> u8 {
        match self {
            Self::V14 => 14,
            Self::V15 => 15,
            Self::V16 => 16,
            Self::V17 => 17,
            Self::V18 => 18,
        }
    }

    pub(crate) fn membership(self) -> MembershipModel {
        match self {
            Self::V14 | Self::V15 => MembershipModel::RoleInheritance,
            Self::V16 | Self::V17 | Self::V18 => MembershipModel::GrantOptions,
        }
    }

    /// PostgreSQL 15 added `pg_parameter_acl` and `GRANT SET ON PARAMETER`.
    pub(crate) fn has_parameter_acl(self) -> bool {
        !matches!(self, Self::V14)
    }

    /// The destructive local reset reads PostgreSQL 17's login event-trigger
    /// flag and per-membership grant options.
    pub(crate) fn supports_local_reset(self) -> bool {
        matches!(self, Self::V17 | Self::V18)
    }
}

impl std::fmt::Display for PostgresMajor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PostgreSQL {}", self.number())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_finite_supported_majors_are_admitted() {
        for (value, expected) in [
            (140_024, PostgresMajor::V14),
            (150_019, PostgresMajor::V15),
            (160_015, PostgresMajor::V16),
            (170_011, PostgresMajor::V17),
            (180_006, PostgresMajor::V18),
        ] {
            assert_eq!(
                PostgresMajor::from_server_version_num(value).ok(),
                Some(expected)
            );
        }
        for refused in [90_624, 130_022, 139_999, 190_000, 200_001, 0, -1] {
            let error = PostgresMajor::from_server_version_num(refused)
                .expect_err("an unqualified server major must be refused");
            assert!(error.to_string().contains("is not supported"), "{error}");
            assert!(error.to_string().contains("nothing was changed"), "{error}");
        }
        assert_eq!(
            PostgresMajor::SUPPORTED.map(PostgresMajor::number),
            [14, 15, 16, 17, 18]
        );
    }

    #[test]
    fn membership_and_parameter_catalogues_follow_actual_server_semantics() {
        assert_eq!(
            PostgresMajor::V14.membership(),
            MembershipModel::RoleInheritance
        );
        assert_eq!(
            PostgresMajor::V15.membership(),
            MembershipModel::RoleInheritance
        );
        for major in [PostgresMajor::V16, PostgresMajor::V17, PostgresMajor::V18] {
            assert_eq!(major.membership(), MembershipModel::GrantOptions);
        }
        assert!(!PostgresMajor::V14.has_parameter_acl());
        assert!(PostgresMajor::V15.has_parameter_acl());
        assert!(PostgresMajor::V18.has_parameter_acl());
        assert!(!PostgresMajor::V16.supports_local_reset());
        assert!(PostgresMajor::V17.supports_local_reset());
    }
}
