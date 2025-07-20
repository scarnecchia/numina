# Entity System Migration Guide

This guide explains how to migrate from the old foreign key-based system to the new SurrealDB RELATE-based entity system.

## Overview

The Pattern entity system has been refactored to use SurrealDB's native graph relationships instead of foreign keys. This provides:

- **Automatic relationship management** - SurrealDB handles bidirectional traversal
- **Relationship metadata** - Edge tables can store additional properties
- **Type safety** - Compile-time validation with macros
- **Cleaner entities** - No more foreign key fields cluttering your structs

## Key Changes

### 1. Entity Field Changes

**Before:**
```rust
pub struct Task {
    pub id: TaskId,
    pub owner_id: UserId,              // ❌ Foreign key
    pub assigned_agent_id: Option<AgentId>, // ❌ Foreign key
    pub parent_task_id: Option<TaskId>,     // ❌ Foreign key
    // ... other fields
}
```

**After:**
```rust
define_entity! {
    task Task {
        status: TaskStatus,
        priority: TaskPriority,
        // No foreign keys! Relationships handled by RELATE
    }
}
```

### 2. ID Types

All ID fields now use SurrealDB's `record` type:

```sql
-- Before
DEFINE FIELD id ON task TYPE string;
DEFINE FIELD owner_id ON task TYPE string;

-- After  
DEFINE FIELD id ON task TYPE record;
-- No owner_id field! Use edge tables instead
```

### 3. Relationships via Edge Tables

Instead of foreign keys, we use RELATE to create edge tables:

| Old Foreign Key | New Edge Table | Query Pattern |
|----------------|----------------|---------------|
| `user.id` → `agent.owner_id` | `user->owns->agent` | `SELECT ->owns->agent FROM user:id` |
| `user.id` → `task.owner_id` | `user->created->task` | `SELECT ->created->task FROM user:id` |
| `agent.id` → `task.assigned_agent_id` | `agent->assigned->task` | `SELECT ->assigned->task FROM agent:id` |
| `task.id` → `task.parent_task_id` | `task->subtask_of->task` | `SELECT ->subtask_of->task FROM task:id` |
| `user.id` → `memory.owner_id` | `user->remembers->mem` | `SELECT ->remembers->mem FROM user:id` |
| `user.id` → `event.owner_id` | `user->scheduled->event` | `SELECT ->scheduled->event FROM user:id` |

## Migration Steps

### 1. Update Entity Definitions

Use the `define_entity!` macro for all your entities:

```rust
use pattern_core::define_entity;

// Base entities (no extensions)
define_entity! {
    user User {}
}

// Entities with custom fields
define_entity! {
    task ADHDTask {
        status: ADHDTaskStatus,
        priority: ADHDTaskPriority,
        // ADHD-specific fields
        time_multiplier: f32,
        energy_required: EnergyLevel,
    }
}
```

### 2. Update Creation Code

**Before:**
```rust
let task = Task {
    id: TaskId::generate(),
    title: "Review PR".to_string(),
    owner_id: user.id.clone(),
    assigned_agent_id: Some(agent.id.clone()),
    // ... other fields
};
db.create_task(task).await?;
```

**After:**
```rust
// Create the task without foreign keys
let task = Task {
    id: TaskId::generate(),
    title: "Review PR".to_string(),
    // ... other fields (no owner_id or assigned_agent_id)
};

// Create the entity
let task = create_entity(&db, &task).await?;

// Create relationships
relate_user_created_task(&db, &user.id, &task.id).await?;
relate_agent_assigned_task(&db, &agent.id, &task.id, Some("high".to_string())).await?;
```

### 3. Update Query Code

**Before:**
```rust
// Get all tasks for a user
let tasks = db.query(
    "SELECT * FROM task WHERE owner_id = $user_id"
).bind(("user_id", user.id)).await?;

// Get assigned agent
let agent_id = task.assigned_agent_id;
```

**After:**
```rust
// Get all tasks created by a user
let tasks = get_user_tasks(&db, &user.id).await?;

// Or use raw RELATE queries
let tasks = db.query(
    "SELECT ->created->task FROM user:$user_id"
).bind(("user_id", user.id)).await?;

// Get assigned agent
let agents = db.query(
    "SELECT <-assigned<-agent FROM task:$task_id"
).bind(("task_id", task.id)).await?;
```

### 4. Available Helper Functions

The system provides helper functions for common operations:

```rust
use pattern_core::db::ops::{
    // Relationship creation
    relate_user_owns_agent,
    relate_user_created_task,
    relate_agent_assigned_task,
    
    // Queries
    get_user_agents,      // Get all agents owned by a user
    get_user_tasks,       // Get all tasks created by a user
    get_agent_tasks,      // Get all tasks assigned to an agent
    get_agent_owner,      // Get the owner of an agent
};
```

### 5. Custom Relationships

You can create your own relationships with metadata:

```rust
// Create a custom relationship with properties
let props = serde_json::json!({
    "started_at": Utc::now(),
    "priority": "high",
    "estimated_hours": 2.5,
});

create_relation(
    &db,
    "agent",
    &agent.id.to_string(),
    "working_on",     // Custom relation name
    "task",
    &task.id.to_string(),
    Some(props),
).await?;

// Query the relationship
let tasks = db.query(
    "SELECT ->working_on->task FROM agent:$agent_id 
     WHERE ->working_on.priority = 'high'"
).bind(("agent_id", agent.id)).await?;
```

## Data Migration Script

To migrate existing data, run these SurrealDB queries:

```sql
-- Create relationships from existing foreign keys

-- User owns Agent
LET $agents = (SELECT id, owner_id FROM agent WHERE owner_id);
FOR $agent IN $agents {
    RELATE ($agent.owner_id)->owns->($agent.id) 
    SET created_at = time::now();
};

-- User created Task  
LET $tasks = (SELECT id, owner_id FROM task WHERE owner_id);
FOR $task IN $tasks {
    RELATE ($task.owner_id)->created->($task.id)
    SET created_at = time::now();
};

-- Agent assigned to Task
LET $assignments = (SELECT id, assigned_agent_id FROM task WHERE assigned_agent_id);
FOR $task IN $assignments {
    RELATE ($task.assigned_agent_id)->assigned->($task.id)
    SET assigned_at = time::now();
};

-- Task subtask relationships
LET $subtasks = (SELECT id, parent_task_id FROM task WHERE parent_task_id);
FOR $task IN $subtasks {
    RELATE ($task.id)->subtask_of->($task.parent_task_id)
    SET created_at = time::now();
};

-- After verifying relationships work, remove old fields
ALTER TABLE agent DROP FIELD owner_id;
ALTER TABLE task DROP FIELD owner_id;
ALTER TABLE task DROP FIELD assigned_agent_id;
ALTER TABLE task DROP FIELD parent_task_id;
-- etc...
```

## Benefits of the New System

1. **Bidirectional Queries** - Navigate relationships in both directions
2. **Relationship History** - Edge tables can track when relationships were created/modified
3. **Metadata Storage** - Store additional data on relationships (priority, timestamps, etc.)
4. **Cleaner Entities** - No foreign key clutter in your domain models
5. **Type Safety** - Compile-time validation with macros
6. **Graph Traversal** - Leverage SurrealDB's graph query capabilities

## Common Patterns

### Finding Related Entities

```rust
// Find all tasks for a user (through any relationship)
let query = "
    SELECT ->created->task AS created_tasks,
           ->owns->agent->assigned->task AS assigned_tasks
    FROM user:$user_id
";

// Find task hierarchy
let query = "
    SELECT id, title,
           ->subtask_of->task AS subtasks,
           <-subtask_of<-task AS parent
    FROM task:$task_id
";
```

### Filtering Relationships

```rust
// Get high-priority assigned tasks
let query = "
    SELECT ->assigned->task FROM agent:$agent_id
    WHERE ->assigned.priority = 'high'
";

// Get recently created tasks
let query = "
    SELECT ->created->task FROM user:$user_id
    WHERE ->created.created_at > time::now() - 7d
";
```

### Complex Traversals

```rust
// Find all tasks assigned to agents owned by a user
let query = "
    SELECT ->owns->agent->assigned->task 
    FROM user:$user_id
";

// Find users who have tasks assigned to a specific agent
let query = "
    SELECT <-created<-user 
    FROM task
    WHERE ->assigned->agent = agent:$agent_id
";
```

## Troubleshooting

### Issue: Old code expects foreign key fields

**Solution**: Update the code to use relationship queries instead of direct field access.

### Issue: Performance concerns with relationship queries

**Solution**: SurrealDB optimizes graph traversals. For hot paths, consider:
- Creating indexes on edge tables
- Caching frequently accessed relationships
- Using `FETCH` to include related data in initial query

### Issue: Need to store relationship metadata

**Solution**: Add fields to the RELATE statement:
```rust
create_relation(&db, "user", &user_id, "created", "task", &task_id,
    Some(json!({
        "created_at": Utc::now(),
        "created_by_agent": agent_id,
        "initial_priority": "high"
    }))
).await?;
```

## Next Steps

1. Update your entity definitions to use `define_entity!`
2. Remove foreign key fields from your structs
3. Update creation code to use RELATE
4. Update queries to traverse relationships
5. Run migration script on existing data
6. Test thoroughly!

For more examples, see the test cases in `pattern_core/src/db/entity/base.rs`.