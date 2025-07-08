//! Utility functions and helpers for pattern-core

pub mod debug;

pub mod duration_millis {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Option<Duration>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match duration {
            Some(d) => d.as_millis().serialize(serializer),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Duration>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let millis: Option<u64> = Option::deserialize(deserializer)?;
        Ok(millis.map(Duration::from_millis))
    }
}

pub mod duration_secs {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Option<Duration>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match duration {
            Some(d) => d.as_secs().serialize(serializer),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Duration>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let secs: Option<u64> = Option::deserialize(deserializer)?;
        Ok(secs.map(Duration::from_secs))
    }
}

/// Format a duration in a human-readable way
pub fn format_duration(duration: std::time::Duration) -> String {
    let total_secs = duration.as_secs();
    let days = total_secs / 86400;
    let hours = (total_secs % 86400) / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;
    let millis = duration.subsec_millis();

    let mut parts = Vec::new();

    if days > 0 {
        parts.push(format!("{}d", days));
    }
    if hours > 0 {
        parts.push(format!("{}h", hours));
    }
    if minutes > 0 {
        parts.push(format!("{}m", minutes));
    }
    if seconds > 0 || (parts.is_empty() && millis == 0) {
        parts.push(format!("{}s", seconds));
    }
    if millis > 0 && parts.is_empty() {
        parts.push(format!("{}ms", millis));
    }

    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(Duration::from_secs(0)), "0s");
        assert_eq!(format_duration(Duration::from_millis(500)), "500ms");
        assert_eq!(format_duration(Duration::from_secs(45)), "45s");
        assert_eq!(format_duration(Duration::from_secs(90)), "1m 30s");
        assert_eq!(format_duration(Duration::from_secs(3661)), "1h 1m 1s");
        assert_eq!(format_duration(Duration::from_secs(90061)), "1d 1h 1m 1s");
    }

    #[test]
    fn test_duration_millis_serde() {
        use serde::{Deserialize, Serialize};

        #[derive(Serialize, Deserialize)]
        struct TestStruct {
            #[serde(with = "duration_millis")]
            duration: Option<Duration>,
        }

        let test = TestStruct {
            duration: Some(Duration::from_millis(1500)),
        };

        let json = serde_json::to_string(&test).unwrap();
        assert_eq!(json, r#"{"duration":1500}"#);

        let deserialized: TestStruct = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.duration, Some(Duration::from_millis(1500)));

        let test_none = TestStruct { duration: None };
        let json_none = serde_json::to_string(&test_none).unwrap();
        assert_eq!(json_none, r#"{"duration":null}"#);
    }
}
