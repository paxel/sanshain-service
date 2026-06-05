CREATE TABLE IF NOT EXISTS user_favorites (
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    item_type VARCHAR(50) NOT NULL CHECK(item_type IN ('service', 'client')),
    item_name VARCHAR(255) NOT NULL,
    PRIMARY KEY (user_id, item_type, item_name)
);
