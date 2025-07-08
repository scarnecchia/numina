-- Migration to add agents array to memory blocks
-- This allows for more efficient live queries when watching for memory updates

-- Add the agents field to existing memory blocks
UPDATE mem SET agents = [] WHERE agents IS NONE;

-- Create an index on the agents array for efficient queries
DEFINE INDEX mem_agents ON mem FIELDS agents;

-- Populate the agents array based on existing agent_memories relationships
UPDATE mem SET agents = (
    SELECT array::distinct(in) FROM agent_memories WHERE out = $parent.id
);
