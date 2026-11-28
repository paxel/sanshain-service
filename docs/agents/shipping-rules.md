# Shipping rules

Checked by the `ship-it` skill on every round, in addition to the global rules.

- The site summary is the release's: `site/start/whats-new.html` names the version in Cargo.toml and every "What changed in" box is for that version: `v=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1); grep -q "$v" site/start/whats-new.html && ! grep -rhoE 'What changed in [0-9.]+' site | grep -qv "in $v$"`
