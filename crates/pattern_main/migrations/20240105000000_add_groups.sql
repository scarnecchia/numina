-- Add groups table for Letta group management
CREATE TABLE IF NOT EXISTS groups (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL,
    group_id TEXT NOT NULL,
    name TEXT NOT NULL,
    manager_type INTEGER NOT NULL CHECK(manager_type IN (0, 1, 2, 3, 4, 5)),
    -- 0: round_robin, 1: supervisor, 2: dynamic, 3: sleeptime, 4: voice_sleeptime, 5: swarm
    manager_config TEXT, -- JSON configuration for the manager
    shared_block_ids TEXT, -- JSON array of shared memory block IDs
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (user_id) REFERENCES users(id),
    UNIQUE(user_id, group_id)
);

-- Index for fast lookups
CREATE INDEX idx_groups_user_id ON groups(user_id);
CREATE INDEX idx_groups_group_id ON groups(group_id);

-- Trigger to update updated_at on changes
CREATE TRIGGER update_groups_updated_at
AFTER UPDATE ON groups
BEGIN
    UPDATE groups SET updated_at = CURRENT_TIMESTAMP WHERE id = NEW.id;
END;