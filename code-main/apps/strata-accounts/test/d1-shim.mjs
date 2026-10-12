// Adaptador mínimo con la forma de la API de D1 sobre node:sqlite, solo para los tests de Node.
// En producción la base de datos es la D1 real de Cloudflare.
import { DatabaseSync } from 'node:sqlite'
import { readdirSync, readFileSync } from 'node:fs'

const MIGRATIONS = new URL('../migrations/', import.meta.url)

export function createD1(path = ':memory:') {
	const sqlite = new DatabaseSync(path)
	sqlite.exec('PRAGMA foreign_keys=ON;')
	for (const file of readdirSync(MIGRATIONS).filter((name) => name.endsWith('.sql')).sort()) {
		sqlite.exec(readFileSync(new URL(file, MIGRATIONS), 'utf-8'))
	}

	const statement = (sql, args = []) => ({
		bind: (...bound) => statement(sql, bound),
		first: async () => sqlite.prepare(sql).get(...args) ?? null,
		all: async () => ({ results: sqlite.prepare(sql).all(...args) }),
		run: async () => ({ meta: { changes: sqlite.prepare(sql).run(...args).changes } }),
		_run: () => sqlite.prepare(sql).run(...args),
	})
	return {
		prepare: (sql) => statement(sql),
		// Como en D1, un lote es atómico: o se aplican todas las sentencias o ninguna.
		batch: async (statements) => {
			sqlite.exec('BEGIN')
			try {
				for (const item of statements) item._run()
				sqlite.exec('COMMIT')
			} catch (error) {
				sqlite.exec('ROLLBACK')
				throw error
			}
			return []
		},
		close: () => sqlite.close(),
	}
}
