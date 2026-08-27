-- FND-2 / CPR-43: install the extension the epoch-3 platform relies on.
-- into the dev database. Runs once, on first boot with an empty data volume.
CREATE EXTENSION IF NOT EXISTS vector;
CREATE EXTENSION IF NOT EXISTS btree_gin;
