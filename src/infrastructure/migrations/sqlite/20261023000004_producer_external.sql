-- A Producer the company does not maintain (an outside API whose spec an
-- administrator provided), drawn and tagged as external. Set by an admin
-- provide of a new Producer, or by hand.
ALTER TABLE services ADD COLUMN external INTEGER NOT NULL DEFAULT 0;
