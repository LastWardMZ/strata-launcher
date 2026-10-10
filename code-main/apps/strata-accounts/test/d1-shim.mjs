// Adaptador mínimo con la forma de la API de D1 sobre node:sqlite, solo para los tests de Node.
// En producción la base de datos es la D1 real de Cloudflare.
import { DatabaseSync } from 'node:sqlite'
import { readFileSync } from 'node:fs'

const MIGRATION = new URL('../migrations/0001_init.sql', import.meta.url)

export function createD1(path = ':memory:') {
	const sqlite = new DatabaseSync(path)
	sqlite.exec('PRAGMA foreign_keys=ON;')
	sqlite.exec(readFileSync(MIGRATION, 'utf-8'))

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
