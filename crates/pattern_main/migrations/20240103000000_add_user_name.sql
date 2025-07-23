-- Add name column to users table
ALTER TABLE users ADD COLUMN name TEXT NOT NULL DEFAULT '';

-- Update existing users to use discord_id as initial name
UPDATE users SET name = COALESCE(discord_id, 'user_' || id) WHERE name = '';