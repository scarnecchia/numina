use serenity::{
    all::CreateMessage,
    client::Context,
    model::{
        channel::Message,
        permissions::Permissions,
        id::ChannelId,
        mention::Mentionable,
    },
};
use tracing::{info, warn};

/// Check if the bot has permission to send messages in a channel
pub async fn can_send_in_channel(ctx: &Context, channel_id: ChannelId) -> bool {
    // Try to get channel info
    info!("Checking permissions for channel {}", channel_id);
    match channel_id.to_channel(&ctx).await {
        Ok(channel) => {
            match channel {
                serenity::model::channel::Channel::Guild(guild_channel) => {
                    info!("Channel is a guild channel: {} in guild {}", guild_channel.name, guild_channel.guild_id);
                    // Get current user (bot) from http
                    let current_user = match ctx.http.get_current_user().await {
                        Ok(user) => user.id,
                        Err(e) => {
                            warn!("Failed to get current user: {}", e);
                            return false;
                        }
                    };
                    
                    // Get the full guild (not partial)
                    match ctx.http.get_guild(guild_channel.guild_id).await {
                        Ok(guild) => {
                            // Get member
                            match guild.member(&ctx.http, current_user).await {
                                Ok(member) => {
                                    // Log the bot's roles
                                    info!("Bot member roles in guild {}: {:?}", guild.id, member.roles);
                                    
                                    // Log each role's permissions
                                    for role_id in &member.roles {
                                        if let Some(role) = guild.roles.get(role_id) {
                                            info!("  Role {} ({}): permissions = {:?}", role.name, role.id, role.permissions);
                                        }
                                    }
                                    
                                    // Also check @everyone role (has same ID as guild)
                                    use serenity::model::id::RoleId;
                                    let everyone_role_id = RoleId::new(guild.id.get());
                                    if let Some(everyone_role) = guild.roles.get(&everyone_role_id) {
                                        info!("  @everyone role permissions: {:?}", everyone_role.permissions);
                                    }
                                    
                                    let permissions = guild.user_permissions_in(&guild_channel, &member);
                                    
                                    // Check for required permissions
                                    let required = Permissions::SEND_MESSAGES | Permissions::VIEW_CHANNEL;
                                    let has_perms = permissions.contains(required);
                                    
                                    if !has_perms {
                                        info!(
                                            "Missing permissions in channel {}: has {:?}, needs {:?}",
                                            channel_id, permissions, required
                                        );
                                    }
                                    
                                    has_perms
                                }
                                Err(e) => {
                                    warn!("Failed to get member: {}", e);
                                    false
                                }
                            }
                        }
                        Err(e) => {
                            warn!("Failed to get guild: {}", e);
                            false
                        }
                    }
                }
                serenity::model::channel::Channel::Private(_) => {
                    // DMs are always allowed
                    true
                }
                _ => {
                    // Other channel types - assume no permission
                    false
                }
            }
        }
        Err(e) => {
            warn!("Failed to get channel info for {}: {}", channel_id, e);
            false
        }
    }
}

/// Send a message with fallback to DM if channel send fails
pub async fn send_with_fallback(
    ctx: &Context,
    msg: &Message,
    content: &str,
) -> Result<(), String> {
    // First try to send to the channel
    if can_send_in_channel(ctx, msg.channel_id).await {
        match msg.channel_id.say(&ctx.http, content).await {
            Ok(_) => return Ok(()),
            Err(e) => {
                warn!(
                    "Failed to send to channel {} despite having permissions: {}",
                    msg.channel_id, e
                );
            }
        }
    } else {
        warn!(
            "No permission to send in channel {}, attempting DM fallback",
            msg.channel_id
        );
    }
    
    // Try to DM the user instead
    let dm_content = CreateMessage::new().content(content);
    match msg.author.direct_message(&ctx, dm_content).await {
        Ok(_) => {
            info!("Sent message via DM fallback to user {}", msg.author.id);
            
            // Try to notify in channel that we DMed them (if we can at least view the channel)
            let notice = format!(
                "📬 {} I don't have permission to send messages here, so I've sent you a DM instead.",
                msg.author.mention()
            );
            
            // This might also fail, but that's ok
            let _ = msg.channel_id.say(&ctx.http, notice).await;
            
            Ok(())
        }
        Err(e) => {
            warn!(
                "Failed to send DM fallback to user {}: {}",
                msg.author.id, e
            );
            Err(format!("Could not send message: {}", e))
        }
    }
}

/// Check and log permission issues
pub async fn check_permissions(ctx: &Context, channel_id: ChannelId) -> Permissions {
    info!("check_permissions called for channel {}", channel_id);
    if let Ok(channel) = channel_id.to_channel(&ctx).await {
        if let serenity::model::channel::Channel::Guild(guild_channel) = channel {
            info!("Channel {} is in guild {}", guild_channel.name, guild_channel.guild_id);
            // Get current user from http
            let current_user = match ctx.http.get_current_user().await {
                Ok(user) => user.id,
                Err(e) => {
                    warn!("Failed to get current user: {}", e);
                    return Permissions::empty();
                }
            };
            
            // Get the full guild (not partial)
            info!("Fetching guild {} data", guild_channel.guild_id);
            if let Ok(guild) = ctx.http.get_guild(guild_channel.guild_id).await {
                info!("Got guild data, fetching member {}", current_user);
                if let Ok(member) = guild.member(&ctx.http, current_user).await {
                    // Log the bot's roles
                    info!("Bot member roles in guild {}: {:?}", guild.id, member.roles);
                    info!("Bot member nick: {:?}, joined_at: {:?}", member.nick, member.joined_at);
                    
                    // Log each role's permissions
                    for role_id in &member.roles {
                        if let Some(role) = guild.roles.get(role_id) {
                            info!("  Role {} ({}): permissions = {:?}", role.name, role.id, role.permissions);
                        }
                    }
                    
                    // Also check @everyone role (has same ID as guild)
                    use serenity::model::id::RoleId;
                    let everyone_role_id = RoleId::new(guild.id.get());
                    if let Some(everyone_role) = guild.roles.get(&everyone_role_id) {
                        info!("  @everyone role permissions: {:?}", everyone_role.permissions);
                    }
                    
                    let permissions = guild.user_permissions_in(&guild_channel, &member);
                    
                    info!(
                        "Bot permissions in channel {} ({}): {:?}",
                        guild_channel.name,
                        channel_id,
                        permissions
                    );
                    
                    // Log missing critical permissions
                    if !permissions.contains(Permissions::SEND_MESSAGES) {
                        warn!("Missing SEND_MESSAGES permission in channel {}", channel_id);
                    }
                    if !permissions.contains(Permissions::VIEW_CHANNEL) {
                        warn!("Missing VIEW_CHANNEL permission in channel {}", channel_id);
                    }
                    if !permissions.contains(Permissions::READ_MESSAGE_HISTORY) {
                        info!("Missing READ_MESSAGE_HISTORY permission in channel {}", channel_id);
                    }
                    
                    return permissions;
                }
            }
        }
    }
    
    Permissions::empty()
}