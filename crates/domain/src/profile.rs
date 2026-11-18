//! Public profile customization and nullable patches.

use crate::guild::Member;
use crate::user::User;
use crate::validation::FieldError;

pub const PRONOUNS_MAX_LENGTH: usize = 40;
pub const RGB_MAX: u32 = 0x00ff_ffff;

pub fn pronouns(value: &str) -> Result<(), FieldError> {
    if value.trim().chars().count() > PRONOUNS_MAX_LENGTH {
        return Err(FieldError {
            field: "pronouns".into(),
            code: "BASE_TYPE_MAX_LENGTH",
            message: "Pronouns must be 40 characters or fewer.".into(),
        });
    }
    Ok(())
}

pub fn rgb(field: &str, value: u32) -> Result<(), FieldError> {
    if value > RGB_MAX {
        return Err(FieldError {
            field: field.into(),
            code: "BASE_TYPE_MAX",
            message: "Color must be between 0 and 16777215.".into(),
        });
    }
    Ok(())
}

/// Missing leaves the field unchanged; `Some(None)` resets it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppearancePatch {
    pub banner: Option<Option<String>>,
    pub pronouns: Option<Option<String>>,
    pub theme_colors: Option<Option<[u32; 2]>>,
    pub accent_color: Option<Option<u32>>,
}

impl AppearancePatch {
    pub fn apply(&self, user: &mut User) {
        if let Some(value) = &self.banner {
            user.banner.clone_from(value);
        }
        if let Some(value) = &self.pronouns {
            user.pronouns.clone_from(value);
        }
        if let Some(value) = self.theme_colors {
            user.theme_colors = value;
        }
        if let Some(value) = self.accent_color {
            user.accent_color = value;
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemberProfilePatch {
    pub avatar: Option<Option<String>>,
    pub banner: Option<Option<String>>,
    pub bio: Option<Option<String>>,
}

impl MemberProfilePatch {
    pub fn apply(&self, member: &mut Member) {
        if let Some(value) = &self.avatar {
            member.avatar.clone_from(value);
        }
        if let Some(value) = &self.banner {
            member.banner.clone_from(value);
        }
        if let Some(value) = &self.bio {
            member.bio.clone_from(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_and_pronouns_have_public_profile_bounds() {
        assert!(rgb("theme_colors", 0).is_ok());
        assert!(rgb("theme_colors", RGB_MAX).is_ok());
        assert!(rgb("theme_colors", RGB_MAX + 1).is_err());
        assert!(pronouns(&format!(" {} ", "é".repeat(40))).is_ok());
        assert!(pronouns(&"é".repeat(41)).is_err());
        assert!(pronouns(" ").is_ok());
    }
}
