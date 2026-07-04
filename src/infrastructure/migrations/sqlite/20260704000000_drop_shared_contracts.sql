-- The legacy shared-contract mechanism was removed (ai/improvements.md item #19).
-- Its original multi-publisher purpose had been defunct since contracts were
-- scoped by (branch_name, service_id); the replacement is the message-level
-- AsyncAPI channel contract design (item #20).
DROP TABLE IF EXISTS shared_contracts;
