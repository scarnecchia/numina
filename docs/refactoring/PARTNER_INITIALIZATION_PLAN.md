# Partner Constellation Pre-Initialization Plan

## Problem Statement
Currently, agent constellations are created on first Discord message, causing significant delays:
- Agent creation (6 agents × ~1-2s each)
- Memory block creation (3 blocks × ~0.5s each)
- Group creation (4 groups × ~2-3s each)
- Total initialization time: 20-30 seconds

## Proposed Solution
Pre-create partner constellations at boot time for designated partners.

## Implementation Plan

### Phase 1: Configuration Changes

1. **Update pattern.toml structure**:
```toml
[discord]
token = "..."
application_id = ...

[partners]
# Define partners who get pre-initialized constellations
[[partners.users]]
discord_id = "549170854458687509"
name = "primary_partner"
auto_initialize = true

[[partners.users]]
discord_id = "123456789012345678" 
name = "secondary_partner"
auto_initialize = false  # Can be initialized via slash command
```

2. **Add to Config struct**:
```rust
#[derive(Debug, Clone, Deserialize)]
pub struct PartnersConfig {
    pub users: Vec<PartnerUser>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PartnerUser {
    pub discord_id: String,
    pub name: String,
    pub auto_initialize: bool,
}
```

### Phase 2: Boot-Time Initialization

1. **Modify PatternService::start()**:
   - After multi-agent system init, check for partners config
   - For each partner with `auto_initialize = true`:
     - Create database user if not exists
     - Initialize full constellation (agents, memory, groups)
   - Log initialization progress

2. **Add method to MultiAgentSystem**:
```rust
pub async fn initialize_partner(&self, discord_id: &str, name: &str) -> Result<UserId> {
    // Get or create database user
    let user = self.db.get_or_create_user_by_discord_id(
        discord_id.parse()?, 
        name
    ).await?;
    
    let user_id = UserId(user.id);
    
    // Initialize constellation
    self.initialize_user(user_id).await?;
    
    info!("Partner {} initialized with user_id {}", name, user_id.0);
    Ok(user_id)
}
```

### Phase 3: Slash Command for Partner Management

1. **Add new slash commands**:
   - `/partner add @user` - Add someone as a partner and initialize their constellation
   - `/partner remove @user` - Remove partner status (keeps agents)
   - `/partner list` - Show all configured partners and their status

2. **Implement partner management**:
```rust
async fn handle_partner_command(&self, ctx: &Context, command: &CommandInteraction) {
    let subcommand = // parse subcommand
    
    match subcommand {
        "add" => {
            let mentioned_user = // get mentioned user
            let discord_id = mentioned_user.id.to_string();
            
            // Add to database
            self.db.add_partner(&discord_id, &mentioned_user.name).await?;
            
            // Initialize constellation
            self.multi_agent_system.initialize_partner(&discord_id, &mentioned_user.name).await?;
            
            // Update config file (optional - for persistence across restarts)
        }
        // ... other subcommands
    }
}
```

### Phase 4: Optimize Discord Message Handling

1. **Fast path for initialized partners**:
```rust
async fn process_message(&self, ctx: &Context, msg: &Message) -> Result<String> {
    let state = self.state.read().await;
    
    // Get database user
    let db_user = state.db
        .get_or_create_user_by_discord_id(msg.author.id.get(), &msg.author.name)
        .await?;
    
    let user_id = UserId(db_user.id);
    
    // Check if already initialized (fast path)
    if state.multi_agent_system.is_user_initialized(user_id).await {
        // Skip initialization, go straight to message processing
        return state.multi_agent_system.send_message_to_agent(...).await;
    }
    
    // Slow path: initialize on demand for non-partners
    state.multi_agent_system.initialize_user(user_id).await?;
    // ... continue as before
}
```

2. **Add initialization check method**:
```rust
impl MultiAgentSystem {
    pub async fn is_user_initialized(&self, user_id: UserId) -> bool {
        // Check if user has agents in cache
        self.user_agents.read().await.contains_key(&user_id)
    }
}
```

### Phase 5: Database Schema Updates

1. **Add partners table** (optional - for persistence):
```sql
CREATE TABLE partners (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL,
    added_by INTEGER,
    added_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (user_id) REFERENCES users(id),
    FOREIGN KEY (added_by) REFERENCES users(id),
    UNIQUE(user_id)
);
```

### Phase 6: Performance Monitoring

1. **Add timing logs**:
   - Boot-time initialization duration
   - Per-message processing time
   - Cache hit/miss rates

2. **Metrics to track**:
   - Time from Discord message to agent response
   - Number of pre-initialized vs on-demand initializations
   - Memory usage with pre-initialized constellations

## Benefits

1. **Immediate Response**: Partners get instant responses without initialization delay
2. **Predictable Load**: Heavy initialization happens at boot, not during user interaction
3. **Better UX**: No more "initializing..." messages for partners
4. **Scalability**: Can control which users get pre-initialized constellations

## Migration Path

1. Deploy with auto_initialize = false for all partners
2. Test with single partner via slash command
3. Enable auto_initialize for primary partners
4. Monitor performance and adjust

## Future Enhancements

1. **Lazy agent loading**: Initialize core agents first, specialists on demand
2. **Agent hibernation**: Unload inactive agents after timeout
3. **Warm standby pool**: Pre-create generic agents that can be assigned to users
4. **Constellation templates**: Different agent sets for different partner types