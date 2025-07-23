# Memory Permissions Design

## Permission Levels (Most to Least Restrictive)

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum MemoryPermission {
    ReadOnly,     // Can only read, no modifications
    Partner,      // Requires permission from partner (owner)
    Human,        // Requires permission from any human
    #[default]
    Append,       // Can add to existing content
    ReadWrite,    // Can modify content freely
    Admin,        // Total control, can delete
}
```

## Block Properties

```rust
pub struct MemoryBlock {
    // ... existing fields ...
    pub memory_type: MemoryType,  // Core, Working, Archival
    pub pinned: bool,             // Can't be swapped out of core
    pub permission: MemoryPermission, // Inherent block permission
}
```

## Relation Properties

```rust
pub struct AgentMemoryRelation {
    // ... existing fields ...
    pub access_level: MemoryPermission, // Relation-specific permission
}
```

## Permission Resolution

**Most restrictive permission wins** between:
- Block's inherent permission
- Relation's access level

Example: If block has `ReadWrite` but relation has `ReadOnly`, effective permission is `ReadOnly`.

## Default Core Blocks

1. **Persona Block**
   - Pinned: true
   - Default Permission: Partner (editable but requires confirmation)
   - Contains agent's identity/personality

2. **Human/Partner Block**
   - Pinned: true
   - Default Permission: ReadWrite
   - Contains information about the user

## Permission Checks for Operations

- **Modify Persona**: Check for Partner permission, prompt user
- **Swap Memory**: Check if block is pinned
- **Delete**: Requires Admin permission
- **Create Core**: Check against max_core_blocks constraint


  - manage_core_memory (append, replace, read operations)
  - manage_archival_memory (insert, search, delete)
  - search_conversations (could include filters for time, participant, etc)
  - interact_with_files (read, search, edit - your existing tools)

   #[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
    pub struct SwapMemoryInput {
        pub swap_out: Vec<String>,  // Labels to move from core to archival
        pub swap_in: Vec<String>,   // Labels to move from archival to core
        pub reason: String,         // Why the swap is needed
    }
