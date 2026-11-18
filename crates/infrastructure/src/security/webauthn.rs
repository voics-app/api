//! WebAuthn verifies the signed client-data origin against explicit RP configuration.

use application::ports::{Ceremony, PasskeyRecord, VerifiedCredential, WebauthnVerifier};
use application::{AppError, AppResult};
use serde_json::Value;
use webauthn_rs::prelude::*;

pub struct PasskeyVerifier(Webauthn);

fn invalid() -> AppError {
    AppError::invalid(
        "credential",
        "INVALID_WEBAUTHN",
        "The passkey response could not be verified.",
    )
}

impl PasskeyVerifier {
    pub fn new(rp_id: &str, origin: &url::Url, name: &str) -> AppResult<Self> {
        let webauthn = WebauthnBuilder::new(rp_id, origin)
            .map_err(AppError::internal)?
            .rp_name(name)
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .map_err(AppError::internal)?;
        Ok(Self(webauthn))
    }
}

fn stored(record: &PasskeyRecord) -> AppResult<Passkey> {
    serde_json::from_value(record.credential.material.clone()).map_err(AppError::internal)
}

fn verified(passkey: Passkey) -> AppResult<VerifiedCredential> {
    let credential: Credential = passkey.clone().into();
    Ok(VerifiedCredential {
        id: passkey.cred_id().as_ref().to_vec(),
        material: serde_json::to_value(passkey).map_err(AppError::internal)?,
        counter: credential.counter,
    })
}

impl WebauthnVerifier for PasskeyVerifier {
    fn start_registration(
        &self,
        user: &domain::user::User,
        existing: &[PasskeyRecord],
    ) -> AppResult<Ceremony> {
        let excluded = existing
            .iter()
            .map(stored)
            .collect::<AppResult<Vec<_>>>()?
            .into_iter()
            .map(|key| key.cred_id().clone())
            .collect();
        // Snowflakes are stable opaque account identities; the UUID occupies the same 64 bits.
        let handle = Uuid::from_u128(u128::from(user.id.value()));
        let (options, state) = self
            .0
            .start_passkey_registration(handle, &user.username, user.display_name(), Some(excluded))
            .map_err(|_| invalid())?;
        Ok(Ceremony {
            public_key: serde_json::to_value(options.public_key).map_err(AppError::internal)?,
            state: serde_json::to_value(state).map_err(AppError::internal)?,
        })
    }

    fn finish_registration(&self, response: Value, state: Value) -> AppResult<VerifiedCredential> {
        let response = serde_json::from_value(response).map_err(|_| invalid())?;
        let state = serde_json::from_value(state).map_err(AppError::internal)?;
        verified(
            self.0
                .finish_passkey_registration(&response, &state)
                .map_err(|_| invalid())?,
        )
    }

    fn start_authentication(&self, credentials: &[PasskeyRecord]) -> AppResult<Ceremony> {
        let keys = credentials
            .iter()
            .map(stored)
            .collect::<AppResult<Vec<_>>>()?;
        let (options, state) = self
            .0
            .start_passkey_authentication(&keys)
            .map_err(|_| invalid())?;
        Ok(Ceremony {
            public_key: serde_json::to_value(options.public_key).map_err(AppError::internal)?,
            state: serde_json::to_value(state).map_err(AppError::internal)?,
        })
    }

    fn finish_authentication(
        &self,
        response: Value,
        state: Value,
        credentials: &[PasskeyRecord],
    ) -> AppResult<VerifiedCredential> {
        let response = serde_json::from_value(response).map_err(|_| invalid())?;
        let state = serde_json::from_value(state).map_err(AppError::internal)?;
        let result = self
            .0
            .finish_passkey_authentication(&response, &state)
            .map_err(|_| invalid())?;
        let record = credentials
            .iter()
            .find(|record| record.credential.id.as_slice() == result.cred_id().as_ref())
            .ok_or_else(invalid)?;
        if (record.credential.counter > 0 || result.counter() > 0)
            && result.counter() <= record.credential.counter
        {
            return Err(invalid());
        }
        let mut key = stored(record)?;
        key.update_credential(&result).ok_or_else(invalid)?;
        verified(key)
    }
}
