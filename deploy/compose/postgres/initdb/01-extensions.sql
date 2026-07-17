-- FND-2: install the extensions the platform relies on (tech plan §1.1)
-- into the dev database. Runs once, on first boot with an empty data volume.
CREATE EXTENSION IF NOT EXISTS vector;
CREATE EXTENSION IF NOT EXISTS age;
CREATE EXTENSION IF NOT EXISTS pgmq;
