use domain::Snowflake;

use crate::context::{Actor, ActorKind, Ports};
use crate::error::{AppError, AppResult};

/// A credential as presented in `Authorization`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Credential<'a> {
    Bearer(&'a str),
    Bot(&'a str),
}

impl<'a> Credential<'a> {
    /// Parses `Bearer <jwt>` / `Bot <token>`; a bare value is treated as a JWT.
    pub fn parse(header: &'a str) -> Self {
        let header = header.trim();
        if let Some(token) = header.strip_prefix("Bot ") {
            Credential::Bot(token.trim())
        } else if let Some(token) = header.strip_prefix("Bearer ") {
            Credential::Bearer(token.trim())
        } else {
            Credential::Bearer(header)
        }
    }
}

/// Validates a user JWT (live session required) or a bot token.
pub async fn execute(ports: &Ports, credential: Credential<'_>) -> AppResult<Actor> {
    match credential {
        Credential::Bearer(jwt) => authenticate_user(ports, jwt).await,
        Credential::Bot(token) => authenticate_bot(ports, token).await,
    }
}

async fn authenticate_user(ports: &Ports, jwt: &str) -> AppResult<Actor> {
    let claims = ports.tokens.verify(jwt)?;
    let session = ports.sessions.find(claims.session_id).await?.ok_or(AppError::Unauthorized)?;
    if session.revoked_at.is_some() || session.user_id != claims.user_id {
        return Err(AppError::Unauthorized);
    }
    Ok(Actor {
        user_id: claims.user_id,
        kind: ActorKind::User { session_id: claims.session_id, device_id: session.device_id.or(claims.device_id) },
    })
}

/// Bot tokens look like `base64url(bot_user_id).<secret>` (P3).
async fn authenticate_bot(ports: &Ports, token: &str) -> AppResult<Actor> {
    let identity = ports
        .applications
        .bot_identity(&ports.secrets.hash(token))
        .await?
        .ok_or(AppError::Unauthorized)?;
    if bot_user_id_prefix(token) != Some(identity.user_id) {
        return Err(AppError::Unauthorized);
    }
    let bot = ports.users.find(identity.user_id).await?.ok_or(AppError::Unauthorized)?;
    if bot.is_suspended(ports.clock.now()) {
        return Err(AppError::Unauthorized);
    }
    Ok(Actor { user_id: identity.user_id, kind: ActorKind::Bot { application_id: identity.application_id } })
}

fn bot_user_id_prefix(token: &str) -> Option<Snowflake> {
    let (prefix, _) = token.split_once('.')?;
    decode_base64url(prefix).and_then(|bytes| String::from_utf8(bytes).ok())?.parse().ok()
}

/// Minimal base64url decoder for the token prefix (no padding).
fn decode_base64url(input: &str) -> Option<Vec<u8>> {
    let value = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        } as u32)
    };
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let (mut buffer, mut bits) = (0u32, 0u32);
    for c in input.trim_end_matches('=').bytes() {
        buffer = (buffer << 6) | value(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// Encodes the bot token prefix: base64url of the decimal user id.
pub fn encode_bot_token_prefix(bot_user_id: Snowflake) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let bytes = bot_user_id.to_string().into_bytes();
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |acc, (i, b)| acc | (*b as u32) << (16 - 8 * i));
        let symbols = chunk.len() + 1;
        for i in 0..symbols {
            out.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_authorization_headers() {
        assert_eq!(Credential::parse("Bot abc.def"), Credential::Bot("abc.def"));
        assert_eq!(Credential::parse("Bearer x.y.z"), Credential::Bearer("x.y.z"));
        assert_eq!(Credential::parse("x.y.z"), Credential::Bearer("x.y.z"));
    }

    #[test]
    fn bot_prefix_round_trips() {
        for id in [1u64, 12, 123, 1234, 1_234_567_890_123_456_789] {
            let prefix = encode_bot_token_prefix(Snowflake::new(id));
            let token = format!("{prefix}.secret");
            assert_eq!(bot_user_id_prefix(&token), Some(Snowflake::new(id)), "{id}");
        }
        assert_eq!(encode_bot_token_prefix(Snowflake::new(123)), "MTIz");
        assert_eq!(bot_user_id_prefix("no-dot"), None);
        assert_eq!(bot_user_id_prefix("!!!.x"), None);
    }
}
