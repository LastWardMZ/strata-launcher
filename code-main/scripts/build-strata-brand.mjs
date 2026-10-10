import { copyFileSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { spawnSync } from 'node:child_process'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const require = createRequire(resolve(root, 'apps/docs/package.json'))
const sharp = require('sharp')
const brand = resolve(root, 'apps/app-frontend/public/brand')
const icons = resolve(root, 'apps/app/icons')
const exports = resolve(root, '../Brand')
mkdirSync(exports, { recursive: true })
const master = readFileSync(resolve(brand, 'strata-icon.svg'))
const mark = readFileSync(resolve(brand, 'strata-mark.svg'), 'utf8')
const light = mark.replace('#B3FBD2', '#188A72').replace('#56E3B0', '#15967E').replace('#1DB1C3', '#137D96').replace('#159F9D', '#0B686D')
writeFileSync(resolve(brand, 'strata-mark-light.svg'), light)

const generated = spawnSync(process.execPath, [
	resolve(root, 'apps/app/node_modules/@tauri-apps/cli/tauri.js'), 'icon',
	resolve(brand, 'strata-icon.svg'), '--output', icons,
], { cwd: resolve(root, 'apps/app'), stdio: 'inherit', windowsHide: true })
if (generated.status !== 0) throw new Error('No se pudieron generar los iconos de Tauri.')
copyFileSync(resolve(icons, 'icon.ico'), resolve(icons, 'favicon.ico'))
await sharp(master).resize(256, 256).png().toFile(resolve(root, 'apps/app-frontend/public/strata-icon.png'))
await sharp(master).resize(1024, 1024).png().toFile(resolve(exports, 'Strata-1024.png'))
copyFileSync(resolve(icons, 'icon.ico'), resolve(exports, 'Strata.ico'))
for (const name of ['strata-icon.svg', 'strata-mark.svg', 'strata-mark-light.svg', 'strata-mark-mono.svg']) copyFileSync(resolve(brand, name), resolve(exports, name))
for (const relative of [
	'packages/assets/branding/logo.svg',
	'packages/assets/branding/modrinth-plus.svg',
	'apps/app-frontend/src/assets/modrinth_app.svg',
	'apps/app/icons/apple.icon/Assets/Strata logo.svg',
]) writeFileSync(resolve(root, relative), mark)
await sharp(master).resize(256, 256).png().toFile(resolve(root, 'apps/app-frontend/src/assets/welcome/modrinth-social-icon.png'))

async function bitmap(svg, destination) {
	const { data, info } = await sharp(Buffer.from(svg)).flatten({ background: '#ffffff' }).removeAlpha().raw().toBuffer({ resolveWithObject: true })
	const stride = Math.ceil(info.width * 3 / 4) * 4
	const out = Buffer.alloc(54 + stride * info.height)
	out.write('BM'); out.writeUInt32LE(out.length, 2); out.writeUInt32LE(54, 10)
	out.writeUInt32LE(40, 14); out.writeInt32LE(info.width, 18); out.writeInt32LE(info.height, 22)
	out.writeUInt16LE(1, 26); out.writeUInt16LE(24, 28); out.writeUInt32LE(stride * info.height, 34)
	for (let y = 0; y < info.height; y++) for (let x = 0; x < info.width; x++) {
		const source = (y * info.width + x) * info.channels
		const target = 54 + (info.height - 1 - y) * stride + x * 3
		out[target] = data[source + 2]; out[target + 1] = data[source + 1]; out[target + 2] = data[source]
	}
	writeFileSync(destination, out)
}
const embeddedIcon = `data:image/svg+xml;base64,${master.toString('base64')}`
const embeddedMark = `data:image/svg+xml;base64,${Buffer.from(light).toString('base64')}`
await bitmap(`<svg xmlns="http://www.w3.org/2000/svg" width="164" height="314">
	<defs><linearGradient id="bg" x2=".4" y2="1"><stop stop-color="#163D3E"/><stop offset="1" stop-color="#0A151F"/></linearGradient></defs>
	<rect width="164" height="314" fill="url(#bg)"/>
	<path d="M-50 292 82 216 214 292M-50 270 82 194 214 270" stroke="#34C7A2" opacity=".12" fill="none"/>
	<image href="${embeddedIcon}" x="30" y="44" width="104" height="104"/>
	<text x="82" y="188" text-anchor="middle" fill="#F3FFF9" font-family="Segoe UI, sans-serif" font-size="30" font-weight="700" letter-spacing="-1">Strata</text>
	<text x="82" y="211" text-anchor="middle" fill="#9ABBB7" font-family="Segoe UI, sans-serif" font-size="12">Minecraft Java</text>
	<path d="M30 278H134" stroke="#47CDAA" opacity=".3"/>
</svg>`, resolve(icons, 'installer-sidebar.bmp'))
await bitmap(`<svg xmlns="http://www.w3.org/2000/svg" width="150" height="57"><rect width="150" height="57" fill="white"/><image href="${embeddedMark}" x="12" y="9" width="35" height="39"/><text x="55" y="37" fill="#142E32" font-family="Segoe UI, sans-serif" font-size="25" font-weight="700" letter-spacing="-.8">Strata</text></svg>`, resolve(icons, 'installer-header.bmp'))

const preview = []
for (const size of [16, 24, 32, 48, 64, 128]) preview.push({ input: await sharp(master).resize(size, size).png().toBuffer(), left: 40 + preview.length * 160, top: 156 - Math.floor(size / 2) })
await sharp({ create: { width: 1040, height: 272, channels: 4, background: '#181C23' } }).composite(preview).png().toFile(resolve(exports, 'Strata-tamanos.png'))
console.log('Marca de Strata exportada en', exports)
