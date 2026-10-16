-- Read-only API tokens: a token handed to an AI / MCP client may read the
-- catalog but never change it. The scope is decided at creation and
-- immutable; existing tokens keep what they always had, full access.
ALTER TABLE api_tokens ADD COLUMN scope TEXT NOT NULL DEFAULT 'full';
