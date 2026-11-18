use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModifyVanityUrlRequest {
    #[serde(default, deserialize_with = "crate::wire::double_option")]
    pub code: Option<Option<String>>,
}

#[derive(Debug, Serialize)]
pub struct VanityUrlDto {
    pub code: Option<String>,
    pub uses: u32,
}

impl From<Option<domain::vanity::VanityInvite>> for VanityUrlDto {
    fn from(invite: Option<domain::vanity::VanityInvite>) -> Self {
        match invite {
            Some(invite) => Self {
                code: Some(invite.code),
                uses: invite.uses,
            },
            None => Self {
                code: None,
                uses: 0,
            },
        }
    }
}
