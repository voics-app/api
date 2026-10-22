//! Synced client preferences (client#123): a sparse JSON object with a closed set of typed keys.
//! `merge` validates a PATCH value and folds it into the stored object key by key.

use serde_json::{Map, Value};

const ACCENTS: [&str; 6] = ["iris", "aurora", "ember", "rose", "citrine", "sky"];
const MAX_SOUND_TOGGLES: usize = 64;
const MAX_RECENT_REACTIONS: usize = 32;

pub type Preferences = Map<String, Value>;

#[derive(Debug, PartialEq, Eq)]
pub struct PreferenceError {
    pub key: String,
    pub message: &'static str,
}

fn err(key: &str, message: &'static str) -> PreferenceError {
    PreferenceError {
        key: key.to_owned(),
        message,
    }
}

fn is_snowflake(value: &str) -> bool {
    !value.is_empty() && value.len() <= 20 && value.bytes().all(|b| b.is_ascii_digit())
}

fn validate_recent_reaction(item: &Value) -> bool {
    let Some(obj) = item.as_object() else {
        return false;
    };
    if obj
        .keys()
        .any(|k| !matches!(k.as_str(), "id" | "name" | "animated"))
    {
        return false;
    }
    let id_ok = match obj.get("id") {
        Some(Value::Null) => true,
        Some(Value::String(s)) => is_snowflake(s),
        _ => false,
    };
    let name_ok = obj
        .get("name")
        .and_then(Value::as_str)
        .is_some_and(|n| !n.is_empty() && n.chars().count() <= 64);
    let animated_ok = obj.get("animated").is_none_or(Value::is_boolean);
    id_ok && name_ok && animated_ok
}

fn validate(key: &str, value: &Value) -> Result<(), PreferenceError> {
    let ok = match key {
        "accent" => value.as_str().is_some_and(|s| ACCENTS.contains(&s)),
        "reduce_motion"
        | "show_avatars_in_compact"
        | "link_previews"
        | "notify_only_mentions"
        | "notify_sounds" => value.is_boolean(),
        "font_scale" => value.as_f64().is_some_and(|n| (0.75..=1.5).contains(&n)),
        "sound_toggles" => value.as_object().is_some_and(|o| {
            o.len() <= MAX_SOUND_TOGGLES
                && o.iter()
                    .all(|(k, v)| !k.is_empty() && k.len() <= 32 && v.is_boolean())
        }),
        "ringtone" => value
            .as_str()
            .is_some_and(|s| !s.is_empty() && s.len() <= 32),
        "skin_tone" => value.as_str().is_some_and(|s| s.chars().count() <= 16),
        "recent_reactions" => value.as_array().is_some_and(|a| {
            a.len() <= MAX_RECENT_REACTIONS && a.iter().all(validate_recent_reaction)
        }),
        _ => return Err(err(key, "Unknown preference.")),
    };
    if ok {
        Ok(())
    } else {
        Err(err(key, "Invalid value for this preference."))
    }
}

/// Validates `patch` (must be an object) and returns `current` with its keys replaced.
pub fn merge(current: &Preferences, patch: &Value) -> Result<Preferences, PreferenceError> {
    let patch = patch
        .as_object()
        .ok_or_else(|| err("preferences", "Must be an object."))?;
    let mut next = current.clone();
    for (key, value) in patch {
        validate(key, value)?;
        next.insert(key.clone(), value.clone());
    }
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merges_key_by_key() {
        let current = json!({"accent": "iris", "reduce_motion": true});
        let merged = merge(
            current.as_object().unwrap(),
            &json!({"accent": "sky", "font_scale": 1.25, "notify_sounds": false}),
        )
        .unwrap();
        assert_eq!(
            Value::Object(merged),
            json!({"accent": "sky", "reduce_motion": true, "font_scale": 1.25, "notify_sounds": false})
        );
    }

    #[test]
    fn rejects_unknown_and_bad_values() {
        let empty = Preferences::new();
        for bad in [
            json!({"nope": 1}),
            json!({"accent": "teal"}),
            json!({"font_scale": 3}),
            json!({"reduce_motion": "yes"}),
            json!({"sound_toggles": {"a": 1}}),
            json!({"recent_reactions": [{"id": "x", "name": "a"}]}),
            json!({"recent_reactions": [{"id": null, "name": "a", "extra": 1}]}),
            json!("str"),
        ] {
            assert!(merge(&empty, &bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn accepts_emoji_prefs() {
        let ok = json!({
            "skin_tone": "\u{1F3FD}",
            "ringtone": "classic",
            "recent_reactions": [{"id": null, "name": "\u{1F44D}"}, {"id": "123", "name": "party", "animated": true}],
            "sound_toggles": {"message": false}
        });
        assert!(merge(&Preferences::new(), &ok).is_ok());
    }
}
