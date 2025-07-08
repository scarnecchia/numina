//! Example demonstrating shared memory between agents
//!
//! This example shows how multiple agents can share memory blocks
//! with different access levels, enabling collaborative cognitive support.

use pattern_core::{
    agent::{Agent, AgentState, AgentType, DatabaseAgent, MemoryAccessLevel},
    db::{client, ops::SurrealExt},
    id::UserId,
    llm::MockLlmProvider,
    memory::Memory,
    tool::ToolRegistry,
};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Pattern Shared Memory Example ===\n");

    // Initialize database
    println!("🗄️  Initializing database...");
    let db = Arc::new(client::create_test_db().await?);

    // Create mock LLM provider (in production, use real provider)
    let llm = Arc::new(MockLlmProvider {
        response: "I understand the shared context.".to_string(),
    });

    // Create tool registry
    let tools = Arc::new(ToolRegistry::new());

    // Create a user (the partner who owns the agent constellation)
    println!("👤 Creating user...");
    let user_id = UserId::generate();
    let user = db
        .create_user(
            Some(user_id),
            serde_json::json!({
                "name": "Alex",
                "preferences": {
                    "notification_style": "gentle",
                    "work_hours": "9am-5pm"
                }
            }),
            serde_json::json!({}),
        )
        .await?;

    println!("   User ID: {}", user.id);

    // Create Pattern (orchestrator) agent
    println!("\n🎯 Creating Pattern agent...");
    let pattern_record = db
        .create_agent(
            user.id,
            #[cfg(feature = "nd")]
            AgentType::Pattern,
            #[cfg(not(feature = "nd"))]
            AgentType::Generic,
            "Pattern".to_string(),
            "I am Pattern, your cognitive orchestrator. I coordinate the other agents and ensure you get the support you need.".to_string(),
            serde_json::json!({
                "personality": "supportive",
                "communication_style": "clear and structured"
            }),
            AgentState::Ready,
        )
        .await?;

    // Create Entropy (task breakdown) agent
    println!("🔀 Creating Entropy agent...");
    let entropy_record = db
        .create_agent(
            user.id,
            #[cfg(feature = "nd")]
            AgentType::Entropy,
            #[cfg(not(feature = "nd"))]
            AgentType::Generic,
            "Entropy".to_string(),
            "I am Entropy, your task specialist. I help break down overwhelming tasks into manageable pieces.".to_string(),
            serde_json::json!({
                "specialty": "task_decomposition",
                "approach": "gentle_guidance"
            }),
            AgentState::Ready,
        )
        .await?;

    // Create Momentum (energy tracking) agent
    println!("⚡ Creating Momentum agent...");
    let momentum_record = db
        .create_agent(
            user.id,
            #[cfg(feature = "nd")]
            AgentType::Momentum,
            #[cfg(not(feature = "nd"))]
            AgentType::Generic,
            "Momentum".to_string(),
            "I am Momentum, your energy tracker. I monitor your attention and energy levels throughout the day.".to_string(),
            serde_json::json!({
                "monitoring_interval": "30min",
                "energy_thresholds": {
                    "low": 30,
                    "medium": 60,
                    "high": 80
                }
            }),
            AgentState::Ready,
        )
        .await?;

    // Create shared memory blocks
    println!("\n📝 Creating shared memory blocks...");

    // 1. Current task context (shared between Pattern and Entropy)
    let task_memory = db
        .create_memory(
            None,
            user.id,
            "current_task".to_string(),
            "Working on the quarterly report - feeling overwhelmed by the amount of data to analyze".to_string(),
            Some("Current task the user is struggling with".to_string()),
            serde_json::json!({
                "task_type": "analytical",
                "estimated_hours": 8,
                "deadline": "2024-01-15"
            }),
        )
        .await?;

    // 2. Energy state (shared between Pattern and Momentum)
    let energy_memory = db
        .create_memory(
            None,
            user.id,
            "energy_state".to_string(),
            "Current energy: LOW (been working for 3 hours straight, no breaks)".to_string(),
            Some("User's current energy and attention state".to_string()),
            serde_json::json!({
                "level": 25,
                "last_break": "3 hours ago",
                "focus_duration": 180
            }),
        )
        .await?;

    // 3. Task patterns (shared between all agents)
    let patterns_memory = db
        .create_memory(
            None,
            user.id,
            "task_patterns".to_string(),
            "User tends to procrastinate on large analytical tasks. Works best in 25-minute focused sessions with clear subtasks.".to_string(),
            Some("Observed patterns in how the user handles different types of tasks".to_string()),
            serde_json::json!({
                "effective_strategies": ["pomodoro", "task_breakdown", "visual_progress"],
                "challenging_task_types": ["analytical", "documentation", "planning"]
            }),
        )
        .await?;

    // Set up memory access permissions
    println!("\n🔐 Setting up memory sharing...");

    // Pattern has admin access to all memories (can read, write, and share)
    db.attach_memory_to_agent(pattern_record.id, task_memory.id, MemoryAccessLevel::Admin)
        .await?;
    db.attach_memory_to_agent(
        pattern_record.id,
        energy_memory.id,
        MemoryAccessLevel::Admin,
    )
    .await?;
    db.attach_memory_to_agent(
        pattern_record.id,
        patterns_memory.id,
        MemoryAccessLevel::Admin,
    )
    .await?;

    // Entropy has write access to task memory and read access to patterns
    db.attach_memory_to_agent(entropy_record.id, task_memory.id, MemoryAccessLevel::Write)
        .await?;
    db.attach_memory_to_agent(
        entropy_record.id,
        patterns_memory.id,
        MemoryAccessLevel::Read,
    )
    .await?;

    // Momentum has write access to energy and read access to patterns
    db.attach_memory_to_agent(
        momentum_record.id,
        energy_memory.id,
        MemoryAccessLevel::Write,
    )
    .await?;
    db.attach_memory_to_agent(
        momentum_record.id,
        patterns_memory.id,
        MemoryAccessLevel::Read,
    )
    .await?;

    println!("   ✓ Pattern can access all memories (admin)");
    println!("   ✓ Entropy can update task context");
    println!("   ✓ Momentum can update energy state");
    println!("   ✓ All agents can read task patterns");

    // Initialize agent instances
    println!("\n🤖 Initializing agents...");
    let pattern = Arc::new(
        DatabaseAgent::new(
            pattern_record.id,
            db.clone(),
            llm.clone(),
            None,
            tools.clone(),
        )
        .await?,
    );

    let entropy = Arc::new(
        DatabaseAgent::new(
            entropy_record.id,
            db.clone(),
            llm.clone(),
            None,
            tools.clone(),
        )
        .await?,
    );

    let momentum = Arc::new(
        DatabaseAgent::new(
            momentum_record.id,
            db.clone(),
            llm.clone(),
            None,
            tools.clone(),
        )
        .await?,
    );

    // Demonstrate shared memory in action
    println!("\n🎬 Demonstrating shared memory system...\n");

    // 1. Momentum detects low energy and updates the shared memory
    println!("1️⃣  Momentum detects low energy state");
    let mut energy_update = Memory::new();
    energy_update.create_block(
        "energy_state",
        "CRITICAL: Energy very low. User has been in deep focus for 3+ hours. Immediate break recommended!",
    )?;
    momentum
        .update_memory("energy_state", energy_update)
        .await?;
    println!("   ✓ Momentum updated energy state in shared memory");

    // 2. Pattern reads the updated energy state
    println!("\n2️⃣  Pattern checks energy state");
    let pattern_energy = pattern.get_memory("energy_state").await?.unwrap();
    let energy_block = pattern_energy.get_block("energy_state").unwrap();
    println!("   Pattern sees: {}", energy_block.value);

    // 3. Pattern decides to activate Entropy for task breakdown
    println!("\n3️⃣  Pattern activates Entropy for intervention");

    // 4. Entropy reads both task context and patterns
    println!("\n4️⃣  Entropy analyzes task and patterns");
    let entropy_task = entropy.get_memory("current_task").await?.unwrap();
    let task_block = entropy_task.get_block("current_task").unwrap();
    println!("   Current task: {}", task_block.value);

    let entropy_patterns = entropy.get_memory("task_patterns").await?.unwrap();
    let patterns_block = entropy_patterns.get_block("task_patterns").unwrap();
    println!("   Known patterns: {}", patterns_block.value);

    // 5. Entropy updates task memory with breakdown
    println!("\n5️⃣  Entropy creates task breakdown");
    let mut task_update = Memory::new();
    task_update.create_block(
        "current_task",
        "BREAKDOWN: Quarterly report divided into 4 subtasks:\n\
         1. Data collection (30 min)\n\
         2. Initial analysis (45 min)\n\
         3. Create visualizations (45 min)\n\
         4. Write summary (60 min)\n\
         START WITH: 30-min data collection after a 15-min break!",
    )?;
    entropy.update_memory("current_task", task_update).await?;
    println!("   ✓ Entropy updated task with actionable breakdown");

    // 6. Pattern sees all updates and can coordinate response
    println!("\n6️⃣  Pattern sees all updates");
    let final_task = pattern.get_memory("current_task").await?.unwrap();
    let final_energy = pattern.get_memory("energy_state").await?.unwrap();

    println!("\n📊 Final shared memory state:");
    println!(
        "   Task: {}",
        final_task.get_block("current_task").unwrap().value
    );
    println!(
        "   Energy: {}",
        final_energy.get_block("energy_state").unwrap().value
    );

    // Demonstrate memory access control
    println!("\n🔒 Demonstrating access control:");

    // Entropy cannot access energy memory (no permission)
    let entropy_energy = db
        .get_memory_by_label(entropy_record.id, "energy_state")
        .await?;
    println!(
        "   Entropy access to energy_state: {:?}",
        entropy_energy.is_some()
    );

    // But Pattern can access everything
    let pattern_memories = db.get_agent_memories(pattern_record.id).await?;
    println!(
        "   Pattern has access to {} memory blocks",
        pattern_memories.len()
    );
    for (memory, access) in pattern_memories {
        println!("     - {} ({:?} access)", memory.label, access);
    }

    // Show how this enables cognitive support
    println!("\n✨ Result: Coordinated ADHD support through shared context");
    println!("   - Momentum detected energy depletion");
    println!("   - Pattern orchestrated the response");
    println!("   - Entropy provided task breakdown");
    println!("   - All agents work from shared, up-to-date context");
    println!("   - Privacy maintained through access controls");

    println!("\n🎯 This shared memory architecture enables:");
    println!("   • Real-time coordination between specialized agents");
    println!("   • Context preservation across interactions");
    println!("   • Privacy-aware information sharing");
    println!("   • Scalable multi-agent cognitive support");

    Ok(())
}
