CREATE TABLE IF NOT EXISTS user_favorites (
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    item_type TEXT NOT NULL CHECK(item_type IN ('service', 'client')),
    item_name TEXT NOT NULL,
    PRIMARY KEY (user_id, item_type, item_name)
);
