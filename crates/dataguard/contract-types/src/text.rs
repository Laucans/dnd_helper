//! Validated strings and lists shared by the four contracts, and the serde
//! helpers that tell "absent" from "null".
//!
//! Patterns are checked by hand on ASCII bytes: `[a-z]` and `[0-9]` of the
//! schemas are ASCII-only, and a runtime `Regex::new` would need an `expect`.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};

use crate::ContractError;

/// Deserializes a field that must be present; its type decides on `null`.
///
/// Used on `Option` fields: serde alone turns a missing `Option` into `None`.
pub(crate) fn required<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer)
}

/// Deserializes a present field into `Some`, so `null` is read as a value.
///
/// With `default`, absent is `None`. With `T = Value`, a present `null`
/// stays `Some(Null)`; with any other `T`, `null` is refused.
pub(crate) fn some<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

/// Defines a `String` newtype whose constructor, `TryFrom<String>` and serde
/// impls share one check.
macro_rules! checked_string {
    ($(#[$meta:meta])* $name:ident, $check:path) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            /// Builds the value, or names the rule it breaks.
            ///
            /// # Errors
            ///
            /// Returns a [`ContractError`] when the string is refused.
            pub fn new(value: impl Into<String>) -> Result<Self, ContractError> {
                Self::try_from(value.into())
            }

            /// The validated string.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = ContractError;

            fn try_from(value: String) -> Result<Self, ContractError> {
                $check(&value)?;
                Ok(Self(value))
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

fn non_empty(value: &str, field: &'static str) -> Result<(), ContractError> {
    if value.is_empty() {
        Err(ContractError::Empty { field })
    } else {
        Ok(())
    }
}

fn matches_pattern(
    ok: bool,
    field: &'static str,
    pattern: &'static str,
) -> Result<(), ContractError> {
    if ok {
        Ok(())
    } else {
        Err(ContractError::Pattern { field, pattern })
    }
}

fn check_command_id(value: &str) -> Result<(), ContractError> {
    non_empty(value, "command")
}

fn check_actor(value: &str) -> Result<(), ContractError> {
    non_empty(value, "by")
}

fn check_violation(value: &str) -> Result<(), ContractError> {
    non_empty(value, "violations[]")
}

fn check_data_capability_ref(value: &str) -> Result<(), ContractError> {
    matches_pattern(
        is_data_capability_ref(value),
        "dataCapability",
        r"^[a-z][a-z0-9-]*\.[A-Za-z][A-Za-z0-9]*@[0-9]+$",
    )
}

fn check_reader_ref(value: &str) -> Result<(), ContractError> {
    matches_pattern(
        is_qualified_name(value),
        "readers[]",
        r"^[a-z][a-z0-9-]*\.[A-Za-z][A-Za-z0-9]*$",
    )
}

fn check_partition(value: &str) -> Result<(), ContractError> {
    matches_pattern(is_partition(value), "partition", "^[A-Z][A-Za-z0-9]*/.+$")
}

fn check_ttl(value: &str) -> Result<(), ContractError> {
    matches_pattern(is_ttl(value), "ttl", "^[0-9]+(ms|s|m|h|d)$")
}

/// `[a-z][a-z0-9-]*`
fn is_system(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes.next().is_some_and(|b| b.is_ascii_lowercase())
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// `[A-Za-z][A-Za-z0-9]*`
fn is_name(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes.next().is_some_and(|b| b.is_ascii_alphabetic())
        && bytes.all(|b| b.is_ascii_alphanumeric())
}

/// `<system>.<name>`
fn is_qualified_name(value: &str) -> bool {
    value
        .split_once('.')
        .is_some_and(|(system, name)| is_system(system) && is_name(name))
}

/// `<system>.<name>@<digits>`
fn is_data_capability_ref(value: &str) -> bool {
    value.split_once('@').is_some_and(|(qualified, version)| {
        is_qualified_name(qualified)
            && !version.is_empty()
            && version.bytes().all(|b| b.is_ascii_digit())
    })
}

/// `<Aggregate>/<id>`, the id being non-empty and free of line terminators
/// (the `.` of ECMA-262 does not match them).
fn is_partition(value: &str) -> bool {
    value.split_once('/').is_some_and(|(aggregate, id)| {
        let mut bytes = aggregate.bytes();
        bytes.next().is_some_and(|b| b.is_ascii_uppercase())
            && bytes.all(|b| b.is_ascii_alphanumeric())
            && !id.is_empty()
            && !id.contains(['\n', '\r', '\u{2028}', '\u{2029}'])
    })
}

/// `[0-9]+(ms|s|m|h|d)`
fn is_ttl(value: &str) -> bool {
    let digits = value
        .strip_suffix("ms")
        .or_else(|| value.strip_suffix(['s', 'm', 'h', 'd']));
    digits.is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
}

checked_string!(
    /// The id of a command: an opaque, non-empty string, never parsed.
    CommandId,
    check_command_id
);

checked_string!(
    /// Who authored a command: an opaque, non-empty string.
    Actor,
    check_actor
);

checked_string!(
    /// One readable violation (an invariant id or a reason): non-empty.
    Violation,
    check_violation
);

checked_string!(
    /// A data capability reference, `system.name@version`.
    DataCapabilityRef,
    check_data_capability_ref
);

checked_string!(
    /// A reader reference, `system.name`, without a version.
    ReaderRef,
    check_reader_ref
);

checked_string!(
    /// A queue partition, `Aggregate/id`.
    Partition,
    check_partition
);

checked_string!(
    /// A duration such as `5m` or `24h`, kept as written.
    Ttl,
    check_ttl
);

impl Partition {
    /// The aggregate name, before the first `/`.
    #[must_use]
    pub fn aggregate(&self) -> &str {
        self.0.split_once('/').unwrap_or_default().0
    }

    /// The aggregate id, after the first `/`.
    #[must_use]
    pub fn id(&self) -> &str {
        self.0.split_once('/').unwrap_or_default().1
    }
}

/// A non-empty list of [`Violation`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "Vec<Violation>", into = "Vec<Violation>")]
pub struct Violations(Vec<Violation>);

impl Violations {
    /// Builds the list.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::Empty`] when the list is empty.
    pub fn new(violations: Vec<Violation>) -> Result<Self, ContractError> {
        Self::try_from(violations)
    }

    /// The violations, at least one.
    #[must_use]
    pub fn as_slice(&self) -> &[Violation] {
        &self.0
    }

    /// Gives the violations back.
    #[must_use]
    pub fn into_vec(self) -> Vec<Violation> {
        self.0
    }
}

impl TryFrom<Vec<Violation>> for Violations {
    type Error = ContractError;

    fn try_from(violations: Vec<Violation>) -> Result<Self, ContractError> {
        if violations.is_empty() {
            Err(ContractError::Empty {
                field: "violations",
            })
        } else {
            Ok(Self(violations))
        }
    }
}

impl From<Violations> for Vec<Violation> {
    fn from(violations: Violations) -> Self {
        violations.0
    }
}

/// The non-empty list of recipients of a message (`to`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "Vec<String>", into = "Vec<String>")]
pub struct Recipients(Vec<String>);

impl Recipients {
    /// Builds the list.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::Empty`] when the list is empty.
    pub fn new(recipients: Vec<String>) -> Result<Self, ContractError> {
        Self::try_from(recipients)
    }

    /// The recipients, at least one.
    #[must_use]
    pub fn as_slice(&self) -> &[String] {
        &self.0
    }
}

impl TryFrom<Vec<String>> for Recipients {
    type Error = ContractError;

    fn try_from(recipients: Vec<String>) -> Result<Self, ContractError> {
        if recipients.is_empty() {
            Err(ContractError::Empty { field: "to" })
        } else {
            Ok(Self(recipients))
        }
    }
}

impl From<Recipients> for Vec<String> {
    fn from(recipients: Recipients) -> Self {
        recipients.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partition_pattern() {
        for ok in ["Account/881", "A/x", "Account/a/b", "Account/ é"] {
            assert!(Partition::new(ok).is_ok(), "{ok}");
        }
        for bad in [
            "Account/",
            "account/1",
            "Account",
            "/1",
            "Acc-ount/1",
            "Account/a\nb",
            "Account/a\rb",
            "Account/a\u{2028}b",
            "Account/a\u{2029}b",
        ] {
            assert!(Partition::new(bad).is_err(), "{bad:?}");
        }
        let p = Partition::new("Account/a/b").unwrap();
        assert_eq!((p.aggregate(), p.id()), ("Account", "a/b"));
    }

    #[test]
    fn data_capability_pattern() {
        for ok in ["credit.requestLimitChange@2", "a-1.B@0", "x.y@007"] {
            assert!(DataCapabilityRef::new(ok).is_ok(), "{ok}");
        }
        for bad in [
            "credit.x",
            "credit.x@",
            "credit.x@v2",
            "Credit.x@1",
            "credit.1x@1",
            "credit.x@1@2",
            "credit.x.y@1",
            "-a.b@1",
            "crédit.x@1",
            "credit.x@١",
        ] {
            assert!(DataCapabilityRef::new(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn reader_pattern() {
        assert!(ReaderRef::new("credit.requestLimitChange").is_ok());
        for bad in ["a.b@2", "a", "A.b", "a.", ".b"] {
            assert!(ReaderRef::new(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn ttl_pattern() {
        for ok in ["0s", "24h", "1440m", "5m", "10ms", "1d"] {
            assert!(Ttl::new(ok).is_ok(), "{ok}");
        }
        for bad in [
            "24x", "h", "ms", "", "1", "1 h", "1.5h", "-1h", "1hh", "1sm",
        ] {
            assert!(Ttl::new(bad).is_err(), "{bad:?}");
        }
        assert_ne!(Ttl::new("24h").unwrap(), Ttl::new("1440m").unwrap());
    }

    #[test]
    fn empty_strings_and_lists_are_refused() {
        assert!(CommandId::new("").is_err());
        assert!(Actor::new("").is_err());
        assert!(Violation::new("").is_err());
        assert!(Violations::new(vec![]).is_err());
        assert!(Recipients::new(vec![]).is_err());
    }

    #[test]
    fn errors_do_not_echo_the_value() {
        let message = Partition::new("secret-value").unwrap_err().to_string();
        assert!(message.contains("partition"));
        assert!(!message.contains("secret-value"));
    }
}
