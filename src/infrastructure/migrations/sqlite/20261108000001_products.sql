-- Products (#76, ADR-0011): named envelopes of systems, defined from outside
-- and above. A Product passes no power over its members.
--
-- Names are unique among Products only: a Product may share its name with a
-- system, the `product:` prefix and the separate pages keep them apart.
CREATE TABLE products (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL
);

-- One row per (Product, system name), with exactly one origin: `helm` (from a
-- pasted umbrella Chart.yaml) or `manual` (picked by hand). No foreign key to
-- services/clients: a member whose system loses every role stays a member,
-- shown as "not in Sanshain", like the periphery (ADR-0007).
CREATE TABLE product_members (
    product_id INTEGER NOT NULL REFERENCES products(id) ON DELETE CASCADE,
    system_name TEXT NOT NULL,
    origin TEXT NOT NULL CHECK (origin IN ('helm', 'manual')),
    PRIMARY KEY (product_id, system_name)
);

-- Product Maintainers, mirroring the Producer maintainer tables: a row assigns
-- either a user or a group, never both.
CREATE TABLE product_user_maintainers (
    product_id INTEGER NOT NULL REFERENCES products(id) ON DELETE CASCADE,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    assigned_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (product_id, user_id)
);

CREATE TABLE product_group_maintainers (
    product_id INTEGER NOT NULL REFERENCES products(id) ON DELETE CASCADE,
    group_id INTEGER NOT NULL REFERENCES user_groups(id) ON DELETE CASCADE,
    assigned_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (product_id, group_id)
);

CREATE INDEX idx_product_user_maintainers_user ON product_user_maintainers(user_id);
CREATE INDEX idx_product_group_maintainers_group ON product_group_maintainers(group_id);
