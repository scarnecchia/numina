//! Serde helpers for deserializing SurrealDB's response format

use crate::id::{Id, IdType};
use chrono::{DateTime, Utc};
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;
use std::marker::PhantomData;
use std::str::FromStr;

/// Helper to deserialize SurrealDB's wrapped response format
pub fn deserialize_surreal_response<'de, T, D>(deserializer: D) -> Result<T, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    struct Wrapper {
        #[serde(rename = "Object")]
        object: serde_json::Value,
    }

    let wrapper = Wrapper::deserialize(deserializer)?;
    T::deserialize(wrapper.object).map_err(de::Error::custom)
}

/// Custom deserializer for SurrealDB datetime format
pub fn deserialize_surreal_datetime<'de, D>(deserializer: D) -> Result<DateTime<Utc>, D::Error>
where
    D: Deserializer<'de>,
{
    struct DateTimeVisitor;

    impl<'de> Visitor<'de> for DateTimeVisitor {
        type Value = DateTime<Utc>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a SurrealDB datetime object or RFC3339 string")
        }

        fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
        where
            A: MapAccess<'de>,
        {
            let mut datetime_str = None;

            while let Some(key) = map.next_key::<String>()? {
                match key.as_str() {
                    "Datetime" => {
                        datetime_str = Some(map.next_value::<String>()?);
                    }
                    _ => {
                        let _: serde_json::Value = map.next_value()?;
                    }
                }
            }

            if let Some(dt_str) = datetime_str {
                DateTime::parse_from_rfc3339(&dt_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .map_err(de::Error::custom)
            } else {
                Err(de::Error::custom("missing Datetime field"))
            }
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            DateTime::parse_from_rfc3339(value)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(de::Error::custom)
        }
    }

    deserializer.deserialize_any(DateTimeVisitor)
}

/// Custom deserializer for SurrealDB Thing (ID) format
pub fn deserialize_surreal_id<'de, T, D>(deserializer: D) -> Result<Id<T>, D::Error>
where
    T: IdType,
    D: Deserializer<'de>,
{
    struct IdVisitor<T> {
        _phantom: PhantomData<T>,
    }

    impl<'de, T: IdType> Visitor<'de> for IdVisitor<T> {
        type Value = Id<T>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a SurrealDB Thing object or ID string")
        }

        fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
        where
            A: MapAccess<'de>,
        {
            let mut thing_obj = None;

            while let Some(key) = map.next_key::<String>()? {
                match key.as_str() {
                    "Thing" => {
                        thing_obj = Some(map.next_value::<serde_json::Value>()?);
                    }
                    _ => {
                        let _: serde_json::Value = map.next_value()?;
                    }
                }
            }

            if let Some(thing) = thing_obj {
                if let Some(obj) = thing.as_object() {
                    if let Some(id_value) = obj.get("id") {
                        if let Some(id_obj) = id_value.as_object() {
                            if let Some(string_val) = id_obj.get("String") {
                                if let Some(id_str) = string_val.as_str() {
                                    return Id::<T>::parse(id_str).map_err(de::Error::custom);
                                }
                            }
                        }
                    }
                }
            }

            Err(de::Error::custom("invalid Thing format"))
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Id::<T>::parse(value).map_err(de::Error::custom)
        }
    }

    deserializer.deserialize_any(IdVisitor {
        _phantom: PhantomData,
    })
}

/// Custom deserializer for Option<DateTime<Utc>> in SurrealDB format
pub fn deserialize_surreal_datetime_option<'de, D>(
    deserializer: D,
) -> Result<Option<DateTime<Utc>>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum DateTimeOption {
        Some(#[serde(deserialize_with = "deserialize_surreal_datetime")] DateTime<Utc>),
        None,
    }

    match DateTimeOption::deserialize(deserializer)? {
        DateTimeOption::Some(dt) => Ok(Some(dt)),
        DateTimeOption::None => Ok(None),
    }
}

/// Wrapper type for deserializing SurrealDB responses
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurrealRecord<T> {
    #[serde(flatten)]
    pub data: T,
}

/// Custom deserializer for Option<Id<T>> in SurrealDB format
pub fn deserialize_surreal_id_option<'de, T, D>(deserializer: D) -> Result<Option<Id<T>>, D::Error>
where
    T: IdType,
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum IdOption<T: IdType> {
        Some(#[serde(deserialize_with = "deserialize_surreal_id")] Id<T>),
        None,
    }

    match IdOption::deserialize(deserializer)? {
        IdOption::Some(id) => Ok(Some(id)),
        IdOption::None => Ok(None),
    }
}

/// Helper to extract the inner data from a SurrealDB response
pub fn extract_surreal_record<T>(value: serde_json::Value) -> Result<T, serde_json::Error>
where
    T: for<'de> Deserialize<'de>,
{
    // Check if the value is wrapped in Object { "Object": { ... } }
    if let Some(obj) = value.as_object() {
        if let Some(inner) = obj.get("Object") {
            return serde_json::from_value(inner.clone());
        }
    }

    // Otherwise try to deserialize directly
    serde_json::from_value(value)
}

/// Custom deserializer for SurrealDB Strand (wrapped string) format
pub fn deserialize_surreal_strand<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    struct StrandVisitor;

    impl<'de> Visitor<'de> for StrandVisitor {
        type Value = String;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a SurrealDB Strand object or string")
        }

        fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
        where
            A: MapAccess<'de>,
        {
            let mut strand_value = None;

            while let Some(key) = map.next_key::<String>()? {
                match key.as_str() {
                    "Strand" => {
                        strand_value = Some(map.next_value::<String>()?);
                    }
                    _ => {
                        let _: serde_json::Value = map.next_value()?;
                    }
                }
            }

            strand_value.ok_or_else(|| de::Error::custom("missing Strand field"))
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(value.to_string())
        }
    }

    deserializer.deserialize_any(StrandVisitor)
}

/// Custom deserializer for SurrealDB Bool format
pub fn deserialize_surreal_bool<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    struct BoolVisitor;

    impl<'de> Visitor<'de> for BoolVisitor {
        type Value = bool;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a SurrealDB Bool object or boolean")
        }

        fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
        where
            A: MapAccess<'de>,
        {
            let mut bool_value = None;

            while let Some(key) = map.next_key::<String>()? {
                match key.as_str() {
                    "Bool" => {
                        bool_value = Some(map.next_value::<bool>()?);
                    }
                    _ => {
                        let _: serde_json::Value = map.next_value()?;
                    }
                }
            }

            bool_value.ok_or_else(|| de::Error::custom("missing Bool field"))
        }

        fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(value)
        }
    }

    deserializer.deserialize_any(BoolVisitor)
}

/// Generic deserializer for enums stored as SurrealDB Strand values
pub fn deserialize_surreal_enum<'de, T, D>(deserializer: D) -> Result<T, D::Error>
where
    T: Deserialize<'de> + FromStr,
    T::Err: fmt::Display,
    D: Deserializer<'de>,
{
    struct EnumVisitor<T>(PhantomData<T>);

    impl<'de, T> Visitor<'de> for EnumVisitor<T>
    where
        T: Deserialize<'de> + FromStr,
        T::Err: fmt::Display,
    {
        type Value = T;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a SurrealDB Strand object or string")
        }

        fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
        where
            A: MapAccess<'de>,
        {
            let mut strand_value = None;

            while let Some(key) = map.next_key::<String>()? {
                match key.as_str() {
                    "Strand" => {
                        strand_value = Some(map.next_value::<String>()?);
                    }
                    _ => {
                        let _: serde_json::Value = map.next_value()?;
                    }
                }
            }

            if let Some(s) = strand_value {
                T::from_str(&s).map_err(de::Error::custom)
            } else {
                Err(de::Error::custom("missing Strand field"))
            }
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            T::from_str(value).map_err(de::Error::custom)
        }
    }

    deserializer.deserialize_any(EnumVisitor(PhantomData))
}

/// Custom deserializer for SurrealDB record references (e.g., "users:`user-123`")
pub fn deserialize_surreal_record_ref<'de, T, D>(deserializer: D) -> Result<Id<T>, D::Error>
where
    T: IdType,
    D: Deserializer<'de>,
{
    struct RecordRefVisitor<T> {
        _phantom: PhantomData<T>,
    }

    impl<'de, T: IdType> Visitor<'de> for RecordRefVisitor<T> {
        type Value = Id<T>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a SurrealDB record reference string")
        }

        fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
        where
            A: MapAccess<'de>,
        {
            let mut strand_value = None;

            while let Some(key) = map.next_key::<String>()? {
                match key.as_str() {
                    "Strand" => {
                        strand_value = Some(map.next_value::<String>()?);
                    }
                    _ => {
                        let _: serde_json::Value = map.next_value()?;
                    }
                }
            }

            if let Some(s) = strand_value {
                // Parse "table:`id`" format
                if let Some(colon_pos) = s.find(':') {
                    let id_part = &s[colon_pos + 1..];
                    // Remove backticks if present
                    let id_str = id_part.trim_matches('`');
                    Id::<T>::parse(id_str).map_err(de::Error::custom)
                } else {
                    Id::<T>::parse(&s).map_err(de::Error::custom)
                }
            } else {
                Err(de::Error::custom("missing Strand field"))
            }
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            // Parse "table:`id`" format
            if let Some(colon_pos) = value.find(':') {
                let id_part = &value[colon_pos + 1..];
                // Remove backticks if present
                let id_str = id_part.trim_matches('`');
                Id::<T>::parse(id_str).map_err(de::Error::custom)
            } else {
                Id::<T>::parse(value).map_err(de::Error::custom)
            }
        }
    }

    deserializer.deserialize_any(RecordRefVisitor {
        _phantom: PhantomData,
    })
}

/// Recursively unwrap SurrealDB's nested value format
pub fn unwrap_surreal_value(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            // Check for SurrealDB wrapper types
            if let Some(inner) = map.get("Object") {
                return unwrap_surreal_value(inner);
            } else if let Some(datetime_str) = map.get("Datetime").and_then(|v| v.as_str()) {
                return serde_json::Value::String(datetime_str.to_string());
            } else if let Some(strand_str) = map.get("Strand").and_then(|v| v.as_str()) {
                return serde_json::Value::String(strand_str.to_string());
            } else if let Some(bool_val) = map.get("Bool").and_then(|v| v.as_bool()) {
                return serde_json::Value::Bool(bool_val);
            } else if let Some(thing) = map.get("Thing").and_then(|v| v.as_object()) {
                // Handle Thing (record reference)
                if let Some(id_obj) = thing.get("id").and_then(|v| v.as_object()) {
                    if let Some(id_str) = id_obj.get("String").and_then(|v| v.as_str()) {
                        return serde_json::Value::String(id_str.to_string());
                    }
                }
            } else if let Some(arr) = map.get("Array") {
                if let Some(arr_val) = arr.as_array() {
                    return serde_json::Value::Array(
                        arr_val.iter().map(|v| unwrap_surreal_value(v)).collect(),
                    );
                }
            } else if let Some(num_obj) = map.get("Number").and_then(|v| v.as_object()) {
                // Handle Number type - could be Int or Float
                if let Some(float_val) = num_obj.get("Float").and_then(|v| v.as_f64()) {
                    return serde_json::Value::Number(
                        serde_json::Number::from_f64(float_val)
                            .unwrap_or_else(|| serde_json::Number::from(0)),
                    );
                } else if let Some(int_val) = num_obj.get("Int").and_then(|v| v.as_i64()) {
                    return serde_json::Value::Number(serde_json::Number::from(int_val));
                }
            }

            // Recursively unwrap all values in the object
            let mut new_map = serde_json::Map::new();
            for (key, val) in map {
                new_map.insert(key.clone(), unwrap_surreal_value(val));
            }
            serde_json::Value::Object(new_map)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(|v| unwrap_surreal_value(v)).collect())
        }
        // Other types are returned as-is
        _ => value.clone(),
    }
}

#[cfg(test)]
mod tests {
    // use super::*;
    // use serde_json::json;

    // Commented out since we're not using these custom deserializers anymore
    // We're using unwrap_surreal_value instead

    // #[test]
    // fn test_deserialize_surreal_datetime() {
    //     let wrapped = json!({
    //         "Datetime": "2025-07-07T03:40:47.365534511Z"
    //     });

    //     let dt: DateTime<Utc> = serde_json::from_value(wrapped).unwrap();
    //     assert_eq!(dt.to_rfc3339(), "2025-07-07T03:40:47.365534511+00:00");
    // }

    // #[test]
    // fn test_deserialize_surreal_id() {
    //     use crate::id::UserId;

    //     let wrapped = json!({
    //         "Thing": {
    //             "tb": "users",
    //             "id": {
    //                 "String": "user-ad658037-a264-4121-9fba-bfac9bfffc3f"
    //             }
    //         }
    //     });

    //     let id: UserId = serde_json::from_value(wrapped).unwrap();
    //     assert_eq!(id.to_string(), "user-ad658037-a264-4121-9fba-bfac9bfffc3f");
    // }
}
