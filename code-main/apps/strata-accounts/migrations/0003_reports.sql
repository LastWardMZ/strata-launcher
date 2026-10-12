-- Denuncias de usuarios. No hay claves foráneas a propósito: la denuncia se conserva aunque la cuenta se borre.
-- Se leen con: wrangler d1 execute strata-accounts --remote --command "SELECT * FROM reports ORDER BY created DESC"
CREATE TABLE reports (
	id INTEGER PRIMARY KEY AUTOINCREMENT,
	reporter TEXT NOT NULL,
	target TEXT NOT NULL,
	target_name TEXT NOT NULL,
	reason TEXT NOT NULL,
	created INTEGER NOT NULL
);
CREATE INDEX reports_target ON reports(target);
