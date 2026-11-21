//! Synced client preferences (client#123): a sparse JSON object with a closed set of typed keys.
//! `merge` validates a PATCH value and folds it into the stored object key by key.

use serde_json::{Map, Value};

const ACCENTS: [&str; 6] = ["iris", "aurora", "ember", "rose", "citrine", "sky"];
const ROLE_COLORS: [&str; 3] = ["name", "dot", "hidden"];
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
        | "hide_age_restricted_channels"
        | "show_avatars_in_compact"
        | "link_previews"
        | "notify_only_mentions"
        | "notify_sounds"
        | "link_underline"
        | "high_contrast"
        | "animate_gifs"
        | "animate_emoji"
        | "animate_stickers"
        | "tts_read_aloud" => value.is_boolean(),
        "font_scale" => value.as_f64().is_some_and(|n| (0.75..=1.5).contains(&n)),
        "saturation" => value.as_f64().is_some_and(|n| (0.0..=1.0).contains(&n)),
        "tts_rate" => value.as_f64().is_some_and(|n| (0.5..=2.0).contains(&n)),
        "message_spacing" => value.as_u64().is_some_and(|n| n <= 32),
        "role_colors" => value.as_str().is_some_and(|s| ROLE_COLORS.contains(&s)),
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
        "favorite_gifs" => value.as_array().is_some_and(|a| {
            a.len() <= 100
                && a.iter().all(|v| {
                    v.as_str()
                        .is_some_and(|s| s.starts_with("https://") && s.len() <= 2048)
                })
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
    fn age_restriction_is_an_optional_boolean_preference() {
        let empty = Preferences::new();
        assert!(!merge(&empty, &json!({}))
            .unwrap()
            .contains_key("hide_age_restricted_channels"));
        let hidden = merge(&empty, &json!({"hide_age_restricted_channels": true})).unwrap();
        let visible = merge(&hidden, &json!({"hide_age_restricted_channels": false})).unwrap();
        assert_eq!(visible["hide_age_restricted_channels"], json!(false));
        for value in [json!(null), json!("false"), json!(0), json!({}), json!([])] {
            assert!(merge(&hidden, &json!({"hide_age_restricted_channels": value})).is_err());
        }
    }

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
            json!({"saturation": 1.5}),
            json!({"tts_rate": 0.1}),
            json!({"message_spacing": 40}),
            json!({"message_spacing": 1.5}),
            json!({"role_colors": "bold"}),
            json!({"high_contrast": 1}),
            json!({"sound_toggles": {"a": 1}}),
            json!({"recent_reactions": [{"id": "x", "name": "a"}]}),
            json!({"recent_reactions": [{"id": null, "name": "a", "extra": 1}]}),
            json!("str"),
        ] {
            assert!(merge(&empty, &bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn accepts_appearance_and_accessibility_prefs() {
        let ok = json!({
            "saturation": 0.5, "role_colors": "dot", "link_underline": true, "message_spacing": 8,
            "high_contrast": false, "animate_gifs": false, "animate_emoji": true,
            "animate_stickers": false, "tts_read_aloud": true, "tts_rate": 1.5
        });
        assert!(merge(&Preferences::new(), &ok).is_ok());
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

    #[test]
    fn validates_favorite_gifs() {
        let empty = Preferences::new();
        assert!(merge(
            &empty,
            &json!({"favorite_gifs": ["https://example.test/a.gif"]})
        )
        .is_ok());
        assert!(merge(
            &empty,
            &json!({"favorite_gifs": ["http://example.test/a.gif"]})
        )
        .is_err());
        assert!(merge(
            &empty,
            &json!({"favorite_gifs": vec!["https://example.test/a.gif"; 101]})
        )
        .is_err());
    }
}
