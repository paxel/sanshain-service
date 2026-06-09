INSERT INTO settings (key, value) VALUES ('auth_mode', 'local') ON CONFLICT DO NOTHING;
