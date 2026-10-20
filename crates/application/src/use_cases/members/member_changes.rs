//! Persisting a member change with its event and audit entry.

use serde_json::json;

use domain::audit::{AuditAction, AuditChange};
use domain::guild::{Member, MemberProfile};
use domain::Snowflake;

use crate::context::Ports;
use crate::error::AppResult;
use crate::events::DomainEvent;
use crate::support::audit::AuditContext;
use crate::support::views::member_profile;
use crate::write_set::{Mutation, WriteSet};

pub async fn commit_member_update(
    ports: &Ports,
    before: &Member,
    after: Member,
    voice: Option<Option<Snowflake>>,
    audit: AuditContext<'_>,
) -> AppResult<MemberProfile> {
    let profile = member_profile(ports, after).await?;
    let member = &profile.member;
    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::UpsertMember(member.clone()))
        .emit(DomainEvent::MemberUpdate(profile.clone()));

    let added: Vec<String> = member
        .role_ids
        .iter()
        .filter(|r| !before.role_ids.contains(r))
        .map(ToString::to_string)
        .collect();
    let removed: Vec<String> = before
        .role_ids
        .iter()
        .filter(|r| !member.role_ids.contains(r))
        .map(ToString::to_string)
        .collect();
    if !added.is_empty() || !removed.is_empty() {
        let changes = vec![
            AuditChange::new("$add", None, Some(json!(added))),
            AuditChange::new("$remove", None, Some(json!(removed))),
        ];
        writes.audit(audit.entry(
            ports,
            member.guild_id,
            AuditAction::MemberRoleUpdate,
            Some(member.user_id),
            changes,
        )?);
    }
    let changes: Vec<AuditChange> = [
        AuditChange::if_changed("nick", json!(before.nick), json!(member.nick)),
        AuditChange::if_changed("mute", json!(before.mute), json!(member.mute)),
        AuditChange::if_changed("deaf", json!(before.deaf), json!(member.deaf)),
        AuditChange::if_changed(
            "communication_disabled_until",
            json!(before.communication_disabled_until),
            json!(member.communication_disabled_until),
        ),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !changes.is_empty() {
        writes.audit(audit.entry(
            ports,
            member.guild_id,
            AuditAction::MemberUpdate,
            Some(member.user_id),
            changes,
        )?);
    }
    if let Some(channel_id) = voice {
        let (action, changes) = match channel_id {
            Some(id) => (
                AuditAction::MemberMove,
                vec![AuditChange::new(
                    "channel_id",
                    None,
                    Some(json!(id.to_string())),
                )],
            ),
            None => (AuditAction::MemberDisconnect, Vec::new()),
        };
        writes.emit(DomainEvent::VoiceMemberMove {
            guild_id: member.guild_id,
            user_id: member.user_id,
            channel_id,
        });
        writes.audit(audit.entry(
            ports,
            member.guild_id,
            action,
            Some(member.user_id),
            changes,
        )?);
    }
    ports.unit_of_work.commit(writes).await?;
    Ok(profile)
}
