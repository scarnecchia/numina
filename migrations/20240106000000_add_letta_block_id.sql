-- Add letta_block_id to shared_memory table to track corresponding Letta blocks
ALTER TABLE shared_memory ADD COLUMN letta_block_id TEXT;

-- Add index for faster lookups
CREATE INDEX IF NOT EXISTS idx_shared_memory_letta_block_id ON shared_memory(letta_block_id);