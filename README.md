# Pattern - Agent Platform and Support Constellation

Pattern is two things. 

## Pattern Platform:

The first is a platform for building stateful agents, based on the MemGPT paper, similar to Letta. It's flexible and extensible.

- **Flexible data backend**: Based on SurrealDB, which can be used as an embedded or external database.
- **Memory Tools**: Implements the MemGPTv2 architecture, with versatile tools for agent context management and recall.
- **Agent Protection Tools**: Agent memory and context sections can be protected to stabilize the agent, or set to require consent before alteration.
- **Agent Coordination**: Multiple specialized agents can collaborate and coordinate in a variety of configurations.
- **Multi-user support**: Agents can be configured to have a primary "partner" that they support while interacting with others.
- **Easy to self-host**: The embedded database option plus (nearly) pure rust design makes the platform and tools easy to set up.


## The `Pattern` agent constellation:

The second is a multi-agent cognitive support system designed for the neurodivergent. It uses a multi-agent architecture with shared memory to provide external executive function through specialized cognitive agents.

- **Pattern** (Orchestrator) - Runs background checks every 20-30 minutes for attention drift and physical needs
- **Entropy** - Breaks down overwhelming tasks into manageable atomic units
- **Flux** - Translates between ADHD time and clock time (5 minutes = 30 minutes)
- **Archive** - External memory bank for context recovery and pattern finding
- **Momentum** - Tracks energy patterns and protects flow states
- **Anchor** - Manages habits, meds, water, and basic needs without nagging

### Constellation Features:

- **Three-Tier Memory**: Core blocks, searchable sources, and archival storage
- **Discord Integration**: Natural language interface through Discord bot
- **MCP Server**: Expose agent capabilities via Model Context Protocol
- **Cost-Optimized Sleeptime**: Two-tier monitoring (rules-based + AI intervention)
- **Flexible Group Patterns**: Create any coordination style you need
- **Task Management**: ADHD-aware task breakdown with time multiplication
- **Passive Knowledge Sharing**: Agents share insights via embedded documents

## Documentation

All documentation is organized in the [`docs/`](docs/) directory:

## Neurodivergent-specific Design

Pattern understands that neurodivergent brains are different, not broken:

- **Time Translation**: Automatic multipliers (1.5x-3x) for all time estimates
- **Hidden Complexity**: Recognizes that "simple" tasks are never simple
- **No Shame Spirals**: Validates struggles as logical responses, never suggests "try harder"
- **Energy Awareness**: Tracks attention as finite resource that depletes non-linearly
- **Flow Protection**: Distinguishes productive hyperfocus from harmful burnout
- **Context Recovery**: External memory for "what was I doing?" moments

### Custom Agents

Create custom agent configurations through the builder API or configuration files. See [Architecture docs](docs/architecture/) for details.

## Development

### Project Structure

```
pattern/
├── crates/
│   ├── pattern_cli/      # Command-line testing tool
│   ├── pattern_core/     # Agent framework, memory, tools, coordination
│   ├── pattern_nd/       # Tools and agent personalities specific to the neurodivergent support constellation
│   ├── pattern_mcp/      # MCP server implementation
│   ├── pattern_discord/  # Discord bot integration
│   └── pattern_main/     # Main orchestrator binary (mostly legacy as of yet)
├── docs/                 # Architecture and integration guides
└── CLAUDE.md          # Development reference
```

## Roadmap

### In Progress
- Build-out of the core framework
  - Vector search
  - MCP refactor
  - Discord re-integration
- Re-implementation of the core Pattern constellation
- Command-line tool for chat and debugging

### Planned
- Webapp-based playground environment for platform
- Contract/client tracking for freelancers
- Social memory for birthdays and follow-ups
- Activity monitoring for interruption timing
- Bluesky integration for public accountability

## Acknowledgments

- Inspired by Brandon Sanderson's cognitive multiplicity model in Stormlight Archive
- Designed by someone who gets it - time is fake but deadlines aren't

## License

Pattern is dual-licensed:

- **AGPL-3.0** for open source use - see [LICENSE](LICENSE)
- **Commercial License** available for proprietary applications - contact for details

This dual licensing ensures Pattern remains open for the neurodivergent community while supporting sustainable development. Any use of Pattern in a network service or application requires either compliance with AGPL-3.0 (sharing source code) or a commercial license.
