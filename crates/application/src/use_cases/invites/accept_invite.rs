use domain::guild::{Member, MemberProfile};

use crate::context::{Actor, Ports};
use crate::error::{AppError, AppResult, Denial, Resource};
use crate::events::DomainEvent;
use crate::support::views::{guild_snapshot, require_user};
use crate::use_cases::guilds::create_guild::MAX_GUILDS_PER_USER;
use crate::use_cases::invites::get_invite::{view, InviteView};
use crate::write_set::{Mutation, WriteSet};

/// Joins the invite's guild. Joining a guild one is already in is a no-op.
pub async fn execute(ports: &Ports, actor: &Actor, code: &str) -> AppResult<InviteView> {
    actor.require_user()?;
    let invite = ports.invites.find(code).await?.ok_or(AppError::NotFound(Resource::Invite))?;
    let now = ports.clock.now();
    if !invite.is_usable(now) {
        return Err(AppError::NotFound(Resource::Invite));
    }
    let guild_id = invite.guild_id;
    if ports.guilds.member(guild_id, actor.user_id).await?.is_some() {
        return view(ports, invite).await;
    }
    if ports.guilds.ban(guild_id, actor.user_id).await?.is_some() {
        return Err(AppError::Forbidden(Denial::Banned));
    }
    if ports.guilds.list_for_user(actor.user_id).await?.len() >= MAX_GUILDS_PER_USER {
        return Err(AppError::BadRequest { code: 30001, message: "Maximum number of guilds reached (200)".into() });
    }

    let user = require_user(ports, actor.user_id).await?;
    let member = Member::new(guild_id, actor.user_id, now);
    let profile = MemberProfile { member: member.clone(), user };
    let mut snapshot = guild_snapshot(ports, guild_id).await?;
    snapshot.members.push(profile.clone());
    snapshot.member_count += 1;

    let mut writes = WriteSet::new();
    writes
        .mutate(Mutation::ConsumeInvite(invite.code.clone()))
        .mutate(Mutation::UpsertMember(member))
        .emit(DomainEvent::MemberAdd(profile))
        .emit(DomainEvent::GuildCreate { snapshot: Box::new(snapshot), recipients: vec![actor.user_id] });
    if invite.is_last_use() {
        writes.mutate(Mutation::DeleteInvite(invite.code.clone())).emit(DomainEvent::InviteDelete {
            guild_id,
            channel_id: invite.channel_id,
            code: invite.code.clone(),
        });
    }
    ports.unit_of_work.commit(writes).await?;
    let mut used = invite;
    used.uses += 1;
    view(ports, used).await
}
