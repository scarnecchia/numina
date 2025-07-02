use pattern::agents::MultiAgentSystemBuilder;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    // Create a mock multi-agent system with some test agents
    let mut builder = MultiAgentSystemBuilder::new(
        Arc::new(letta::LettaClient::local().unwrap()),
        Arc::new(pattern::db::Database::new(":memory:").await.unwrap()),
    );
    
    // Add some test agents with different names
    builder.add_agent(
        "pattern".to_string(),
        "Pattern".to_string(),
        "Orchestrator agent".to_string(),
        "You are Pattern, the orchestrator.".to_string(),
        true,
    );
    
    builder.add_agent(
        "chaos".to_string(),  // Different from the default "entropy"
        "Chaos".to_string(),
        "Task management agent".to_string(),
        "You are Chaos, handling tasks.".to_string(),
        false,
    );
    
    builder.add_agent(
        "clock".to_string(),  // Different from the default "flux"
        "Clock".to_string(),
        "Time management agent".to_string(),
        "You are Clock, managing time.".to_string(),
        false,
    );
    
    let system = Arc::new(builder.build());
    
    // Test the parse_agent_routing function
    let discord_bot = pattern::discord::PatternDiscordBot::new(system);
    
    // Test cases
    let test_cases = vec![
        "@chaos help me with tasks",
        "clock: what time is it?",
        "/agent pattern check status",
        "@unknown this should default",
        "regular message with no routing",
    ];
    
    for test in test_cases {
        let (agent, message) = discord_bot.parse_agent_routing(test).await;
        println!("Input: \"{}\"", test);
        println!("  Agent: {:?}", agent);
        println!("  Message: \"{}\"", message);
        println!();
    }
}