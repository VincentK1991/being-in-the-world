-- Ontop connects with a read-only role: the virtual graph can never write to the source database.
CREATE ROLE ontop_ro LOGIN PASSWORD 'ontop-read-only';
GRANT CONNECT ON DATABASE acme TO ontop_ro;
GRANT USAGE ON SCHEMA public TO ontop_ro;
GRANT SELECT ON ALL TABLES IN SCHEMA public TO ontop_ro;
ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT SELECT ON TABLES TO ontop_ro;
