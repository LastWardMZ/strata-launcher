// Pruebas de la lógica de versiones y de elección de archivo de los mapas: `node --test src/helpers/maps.test.mjs`
import assert from 'node:assert/strict'
import test from 'node:test'

import { bestFile, compareVersions, formatBytes, newestVersion, supportsQuickPlayWorld } from './maps.ts'

// Orden de las versiones de Minecraft: de la más nueva a la más antigua.
const ordered = ['26.3', '1.21.8', '1.21', '1.20.1', '1.20', '1.19.4', '1.13', '1.8.9']

test('compara la versión del mapa con la de la instancia', () => {
	assert.equal(compareVersions('1.21', '1.20.1', ordered), 'newer', 'mapa más nuevo: avisa de que puede corromperse')
	assert.equal(compareVersions('26.3', '1.8.9', ordered), 'newer')
	assert.equal(compareVersions('1.20.1', '1.21.8', ordered), 'older', 'mapa más antiguo: Minecraft lo actualizará')
	assert.equal(compareVersions('1.8.9', '26.3', ordered), 'older')
	assert.equal(compareVersions('1.20.1', '1.20.1', ordered), 'same')
	assert.equal(compareVersions('99w99z', '1.20.1', ordered), 'unknown', 'si no se conoce, no se afirma nada')
	assert.equal(compareVersions('1.20.1', 'otra', ordered), 'unknown')
	assert.equal(compareVersions('otra', 'otra', ordered), 'same')
})

test('la versión más nueva de un archivo que sirve para varias', () => {
	assert.equal(newestVersion(['1.20', '1.21', '1.20.1'], ordered), '1.21')
	assert.equal(newestVersion(['1.19.4'], ordered), '1.19.4')
	assert.equal(newestVersion(['desconocida'], ordered), 'desconocida')
	assert.equal(newestVersion([], ordered), null)
})

const file = (id, versions, date, extra = {}) => ({ id, name: `f${id}`, fileName: `f${id}.zip`, size: 1, date, releaseType: 1, gameVersions: versions, available: true, downloadUrl: 'https://edge.forgecdn.net/x.zip', hashes: [], ...extra })

test('elige el archivo más reciente que admita la versión; si no, el más reciente', () => {
	const files = [file(1, ['1.19.4'], '2026-01-01'), file(2, ['1.20.1', '1.20'], '2026-02-01'), file(3, ['1.20.1'], '2026-03-01'), file(4, ['1.21'], '2026-04-01')]
	assert.equal(bestFile(files, '1.20.1').id, 3, 'el más reciente que sirve para 1.20.1')
	assert.equal(bestFile(files, '1.19.4').id, 1)
	assert.equal(bestFile(files, '1.8.9').id, 4, 'ninguno sirve: el más reciente')
	assert.equal(bestFile(files).id, 4)
	assert.equal(bestFile([], '1.20.1'), null)
})

test('no elige archivos no disponibles', () => {
	const files = [file(1, ['1.20.1'], '2026-05-01', { available: false }), file(2, ['1.20.1'], '2026-01-01')]
	assert.equal(bestFile(files, '1.20.1').id, 2)
	assert.equal(bestFile([file(3, ['1.20.1'], '2026-01-01', { available: false })], '1.20.1'), null)
})

test('Quick Play de un mundo existe desde la 1.20', () => {
	assert.equal(supportsQuickPlayWorld('26.3', ordered), true)
	assert.equal(supportsQuickPlayWorld('1.21', ordered), true)
	assert.equal(supportsQuickPlayWorld('1.20', ordered), true)
	assert.equal(supportsQuickPlayWorld('1.19.4', ordered), false, 'antes de la 1.20 solo se abre la instancia')
	assert.equal(supportsQuickPlayWorld('1.8.9', ordered), false)
	assert.equal(supportsQuickPlayWorld('desconocida', ordered), false)
})

test('tamaños legibles', () => {
	assert.equal(formatBytes(100), '1 KB')
	assert.equal(formatBytes(2048), '2 KB')
	assert.equal(formatBytes(5 * 1024 * 1024), '5.0 MB')
	assert.equal(formatBytes(3 * 1024 * 1024 * 1024), '3.00 GB')
})
