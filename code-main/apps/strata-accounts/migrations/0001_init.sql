-- Cuentas de Strata. El id de cada usuario es su clave pública Ed25519 (64 caracteres hexadecimales).
CREATE TABLE users (
	id TEXT PRIMARY KEY,
	username TEXT NOT NULL UNIQUE COLLATE NOCASE,
	bio TEXT NOT NULL DEFAULT '',
	friend_privacy TEXT NOT NULL DEFAULT 'everyone',
	invite_privacy TEXT NOT NULL DEFAULT 'friends',
	searchable INTEGER NOT NULL DEFAULT 1,
	created INTEGER NOT NULL,
	renamed_at INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE relations (
	sender TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
	receiver TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
	status TEXT NOT NULL,
	PRIMARY KEY (sender, receiver)
);
CREATE INDEX relations_receiver ON relations(receiver);

CREATE TABLE blocks (
	owner TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
	target TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
	PRIMARY KEY (owner, target)
);
CREATE INDEX blocks_target ON blocks(target);

-- Límites de uso (se guardan aquí porque cada instancia de un Worker tiene su propia memoria).
CREATE TABLE limits (
	key TEXT PRIMARY KEY,
	count INTEGER NOT NULL,
	until INTEGER NOT NULL
);
