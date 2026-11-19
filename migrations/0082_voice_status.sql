-- Kept separate from channel metadata writes so unrelated edits cannot reset it.
ALTER TABLE channels ADD COLUMN status TEXT
    CHECK (status IS NULL OR char_length(status) <= 500);
