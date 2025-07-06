-- Add agent type to support multiple agents per user
ALTER TABLE agents ADD COLUMN agent_type TEXT;

-- Update existing agents to have a type (if any exist)
UPDATE agents SET agent_type = 'pattern' WHERE agent_type IS NULL;

-- Drop the old unique constraint on letta_agent_id
-- SQLite doesn't support DROP CONSTRAINT, so we need to recreate the table
CREATE TABLE agents_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL,
    agent_type TEXT NOT NULL,
    letta_agent_id TEXT NOT NULL,
    name TEXT NOT NULL,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    UNIQUE(user_id, agent_type)  -- Each user can have one agent of each type
);

-- Copy data from old table, handling duplicates by keeping only the first agent per user
-- This assumes existing agents are 'pattern' type
INSERT INTO agents_new (user_id, agent_type, letta_agent_id, name, created_at, updated_at)
SELECT 
    user_id, 
    COALESCE(agent_type, 'pattern'), 
    letta_agent_id, 
    name, 
    created_at, 
    updated_at
FROM agents
WHERE id IN (
    SELECT MIN(id) FROM agents GROUP BY user_id
);

-- Drop old table and rename new one
DROP TABLE agents;
ALTER TABLE agents_new RENAME TO agents;

-- Recreate indexes
CREATE INDEX idx_agents_user_id ON agents(user_id);
CREATE INDEX idx_agents_letta_agent_id ON agents(letta_agent_id);

-- Recreate trigger
CREATE TRIGGER update_agents_timestamp 
AFTER UPDATE ON agents
BEGIN
    UPDATE agents SET updated_at = CURRENT_TIMESTAMP WHERE id = NEW.id;
END;