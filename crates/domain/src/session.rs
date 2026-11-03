//! What is known about the client behind a login session.

const UNKNOWN: &str = "Unknown";
const MAX_FIELD_CHARS: usize = 64;

/// Client, operating system and approximate location of a login session,
/// derived from request headers when the session starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientInfo {
    pub client: String,
    pub os: String,
    /// An ISO 3166-1 alpha-2 country code, when the edge supplied one.
    pub location: Option<String>,
}

impl Default for ClientInfo {
    fn default() -> Self {
        Self {
            client: UNKNOWN.into(),
            os: UNKNOWN.into(),
            location: None,
        }
    }
}

impl ClientInfo {
    /// Builds the info from a `User-Agent` and the edge's country header
    /// (Cloudflare's `CF-IPCountry`). Anything unrecognised is `Unknown`.
    pub fn from_headers(user_agent: Option<&str>, country: Option<&str>) -> Self {
        let user_agent = user_agent.unwrap_or_default();
        Self {
            client: client_of(user_agent).into(),
            os: os_of(user_agent).into(),
            location: country_code(country),
        }
    }

    /// Restores stored values, bounding their length.
    pub fn restore(client: &str, os: &str, location: Option<&str>) -> Self {
        Self {
            client: client.chars().take(MAX_FIELD_CHARS).collect(),
            os: os.chars().take(MAX_FIELD_CHARS).collect(),
            location: location.map(|code| code.chars().take(2).collect()),
        }
    }
}

fn client_of(user_agent: &str) -> &'static str {
    let contains = |needle: &str| user_agent.contains(needle);
    if contains("Discordy") && contains("Electron/") {
        "Discordy Desktop"
    } else if contains("Discordy") {
        "Discordy"
    } else if contains("Edg/") {
        "Edge"
    } else if contains("OPR/") || contains("Opera") {
        "Opera"
    } else if contains("Firefox/") {
        "Firefox"
    } else if contains("Chrome/") || contains("Chromium/") {
        "Chrome"
    } else if contains("Safari/") {
        "Safari"
    } else {
        UNKNOWN
    }
}

fn os_of(user_agent: &str) -> &'static str {
    let contains = |needle: &str| user_agent.contains(needle);
    if contains("Windows") {
        "Windows"
    } else if contains("Android") {
        "Android"
    } else if contains("iPhone") || contains("iPad") || contains("iOS") {
        "iOS"
    } else if contains("Mac OS X") || contains("Macintosh") {
        "macOS"
    } else if contains("CrOS") {
        "ChromeOS"
    } else if contains("Linux") || contains("X11") {
        "Linux"
    } else {
        UNKNOWN
    }
}

/// Cloudflare reports `XX` (unknown) and `T1` (Tor); neither is a place.
fn country_code(country: Option<&str>) -> Option<String> {
    let code = country?.trim().to_ascii_uppercase();
    let valid = code.len() == 2 && code.bytes().all(|b| b.is_ascii_uppercase());
    (valid && code != "XX" && code != "T1").then_some(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DESKTOP: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Discordy/0.9.0 Chrome/130.0.0.0 Electron/33.0.0 Safari/537.36";
    const FIREFOX_LINUX: &str =
        "Mozilla/5.0 (X11; Linux x86_64; rv:130.0) Gecko/20100101 Firefox/130.0";
    const SAFARI_IPHONE: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1";
    const EDGE: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36 Edg/130.0.0.0";

    fn info(user_agent: &str) -> (String, String) {
        let info = ClientInfo::from_headers(Some(user_agent), None);
        (info.client, info.os)
    }

    #[test]
    fn recognises_the_desktop_app_before_its_chromium_tokens() {
        assert_eq!(info(DESKTOP), ("Discordy Desktop".into(), "Windows".into()));
    }

    #[test]
    fn recognises_common_browsers_and_systems() {
        assert_eq!(info(FIREFOX_LINUX), ("Firefox".into(), "Linux".into()));
        assert_eq!(info(SAFARI_IPHONE), ("Safari".into(), "iOS".into()));
        assert_eq!(info(EDGE), ("Edge".into(), "macOS".into()));
    }

    #[test]
    fn unknown_or_missing_user_agent_is_unknown() {
        assert_eq!(info("curl/8.0"), ("Unknown".into(), "Unknown".into()));
        assert_eq!(ClientInfo::from_headers(None, None), ClientInfo::default());
    }

    #[test]
    fn location_is_a_country_code_or_nothing() {
        let location = |value| ClientInfo::from_headers(None, value).location;
        assert_eq!(location(Some("nl")), Some("NL".into()));
        assert_eq!(location(Some(" DE ")), Some("DE".into()));
        assert_eq!(location(Some("XX")), None);
        assert_eq!(location(Some("T1")), None);
        assert_eq!(location(Some("Netherlands")), None);
        assert_eq!(location(None), None);
    }

    #[test]
    fn restore_bounds_stored_values() {
        let long = "x".repeat(200);
        let info = ClientInfo::restore(&long, &long, Some("NLD"));
        assert_eq!(info.client.chars().count(), MAX_FIELD_CHARS);
        assert_eq!(info.os.chars().count(), MAX_FIELD_CHARS);
        assert_eq!(info.location.as_deref(), Some("NL"));
    }
}
