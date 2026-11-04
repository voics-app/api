//! Scalar wire types: Snowflakes and permission sets travel as decimal strings.

use std::fmt;

use chrono::{DateTime, SecondsFormat, Utc};
use serde::de::{self, Deserializer, Visitor};
use serde::{Deserialize, Serialize, Serializer};

use domain::permissions::Permissions;
use domain::Snowflake;

/// RFC 3339 with millisecond precision, as Discord renders timestamps.
pub fn timestamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// A Snowflake as a decimal string. Deserializes from a string or an integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Id(pub Snowflake);

impl From<Snowflake> for Id {
    fn from(value: Snowflake) -> Self {
        Id(value)
    }
}

impl Serialize for Id {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&self.0)
    }
}

struct IdVisitor;

impl Visitor<'_> for IdVisitor {
    type Value = Id;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a snowflake as a decimal string")
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Id, E> {
        v.parse::<Snowflake>()
            .map(Id)
            .map_err(|_| E::custom("invalid snowflake"))
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Id, E> {
        Ok(Id(Snowflake::new(v)))
    }
}

impl<'de> Deserialize<'de> for Id {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(IdVisitor)
    }
}

/// A permission set as a decimal string. Deserializes from a string or an integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Perm(pub Permissions);

impl From<Permissions> for Perm {
    fn from(value: Permissions) -> Self {
        Perm(value)
    }
}

impl Serialize for Perm {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&self.0.bits())
    }
}

struct PermVisitor;

impl Visitor<'_> for PermVisitor {
    type Value = Perm;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a permission bitset as a decimal string")
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Perm, E> {
        v.parse::<u64>()
            .map(|bits| Perm(Permissions::from_bits_truncate(bits)))
            .map_err(|_| E::custom("invalid permissions"))
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Perm, E> {
        Ok(Perm(Permissions::from_bits_truncate(v)))
    }
}

impl<'de> Deserialize<'de> for Perm {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(PermVisitor)
    }
}

/// Distinguishes an absent field (`None`) from an explicit `null` (`Some(None)`).
pub fn double_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize, PartialEq)]
    struct Body {
        id: Id,
        #[serde(default, deserialize_with = "double_option")]
        parent: Option<Option<Id>>,
        perms: Option<Perm>,
    }

    #[test]
    fn ids_accept_strings_and_integers_and_serialize_as_strings() {
        let body: Body = serde_json::from_str(r#"{"id":"123","parent":456,"perms":"8"}"#).unwrap();
        assert_eq!(body.id, Id(Snowflake::new(123)));
        assert_eq!(body.parent, Some(Some(Id(Snowflake::new(456)))));
        assert_eq!(body.perms, Some(Perm(Permissions::ADMINISTRATOR)));
        assert_eq!(serde_json::to_string(&body.id).unwrap(), "\"123\"");
        assert_eq!(
            serde_json::to_string(&Perm(Permissions::VIEW_CHANNEL)).unwrap(),
            "\"1024\""
        );
    }

    #[test]
    fn double_option_tells_null_from_absent() {
        let absent: Body = serde_json::from_str(r#"{"id":"1"}"#).unwrap();
        let null: Body = serde_json::from_str(r#"{"id":"1","parent":null}"#).unwrap();
        assert_eq!(absent.parent, None);
        assert_eq!(null.parent, Some(None));
    }

    #[test]
    fn rejects_garbage_ids() {
        assert!(serde_json::from_str::<Id>("\"abc\"").is_err());
        assert!(serde_json::from_str::<Id>("true").is_err());
    }

    #[test]
    fn timestamps_use_milliseconds_and_z() {
        let at = DateTime::parse_from_rfc3339("2026-10-17T12:00:00.123456Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(timestamp(at), "2026-10-17T12:00:00.123Z");
    }
}
