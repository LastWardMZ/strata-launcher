import assert from 'node:assert/strict'
import test from 'node:test'

import { renderExternalHtml } from './external-html.ts'

const forbidden = [/<script/i, /<iframe/i, /<style/i, /\son\w+\s*=/i, /javascript:/i, /\sstyle=/i, /<svg/i, /<object/i, /<embed/i, /<form/i, /<link/i]

function assertSafe(html: string) {
	const out = renderExternalHtml(html)
	for (const pattern of forbidden) assert.doesNotMatch(out, pattern, `${pattern} en «${out}» (de «${html}»)`)
	return out
}

test('quita scripts, iframes y estilos con su contenido', () => {
	const out = assertSafe('<p>Hola</p><script>alert(1)</script><iframe src="https://evil.example"></iframe><style>body{display:none}</style><p>fin</p>')
	assert.equal(out, '<p>Hola</p><p>fin</p>')
})

test('quita eventos, estilos, clases e ids de cualquier etiqueta', () => {
	const out = assertSafe('<p onclick="x()" style="color:red" class="a" id="b" onmouseover="y()">texto</p><img src="https://media.forgecdn.net/a.png" onerror="alert(1)" style="x:y" class="z">')
	assert.equal(out, '<p>texto</p><img src="https://media.forgecdn.net/a.png">')
})

test('enlaces: solo http, https y mailto, y siempre se abren fuera', () => {
	assert.equal(assertSafe('<a href="https://www.curseforge.com/minecraft/worlds/x">ver</a>'), '<a rel="noopener nofollow ugc" target="_blank" href="https://www.curseforge.com/minecraft/worlds/x">ver</a>')
	for (const bad of ['javascript:alert(1)', 'JaVaScRiPt:alert(1)', ' javascript:alert(1)', 'data:text/html,<script>alert(1)</script>', 'vbscript:x', 'file:///C:/Windows/win.ini', '//evil.example/x', 'java\tscript:alert(1)']) {
		const out = assertSafe(`<a href="${bad}">x</a>`)
		assert.doesNotMatch(out, /href=/i, `no debe conservar «${bad}»: ${out}`)
	}
	assert.match(assertSafe('<a href="mailto:autor@example.com">correo</a>'), /href="mailto:autor@example.com"/)
	assert.doesNotMatch(assertSafe('<a href="https://x.example" target="_self" onclick="x()">x</a>'), /target="_self"|onclick/)
})

test('imágenes: solo del CDN de CurseForge por https; el resto se descarta (nada de rastreadores)', () => {
	assert.match(renderExternalHtml('<img src="https://media.forgecdn.net/attachments/1/2/a.png" alt="foto" width="300" height="200">'), /src="https:\/\/media\.forgecdn\.net\/attachments\/1\/2\/a\.png"/)
	for (const bad of ['https://i.imgur.com/a.png', 'http://media.forgecdn.net/a.png', 'https://forgecdn.net.evil.com/a.png', 'https://evilforgecdn.net/a.png', 'data:image/svg+xml,<svg onload=alert(1)>', 'javascript:alert(1)', '//media.forgecdn.net/a.png', 'https://wsrv.nl/?url=https://media.forgecdn.net/a.png']) {
		const out = assertSafe(`<img src="${bad}">`)
		assert.doesNotMatch(out, /src=/i, `no debe conservar «${bad}»: ${out}`)
	}
	assert.doesNotMatch(renderExternalHtml('<img src="https://media.forgecdn.net/a.png" width="100%" height="javascript:1">'), /width|height/)
})

test('etiquetas desconocidas se quitan pero el texto se conserva; los cuerpos peligrosos no', () => {
	assert.equal(assertSafe('<custom-tag>texto <b>negrita</b></custom-tag>'), 'texto <b>negrita</b>')
	// El filtro puede dejar algún marcador suelto («[removed]»), pero ninguna etiqueta ni nada que se ejecute.
	for (const body of [
		'<svg><script>alert(1)</script><text>x</text></svg>ok',
		'<math><mi>x</mi></math>ok',
		'<form action="https://evil.example"><input name="pw"><button>enviar</button></form>ok',
		'<video src="https://evil.example/v.mp4"></video><audio src="x"></audio>ok',
	]) {
		const out = assertSafe(body)
		assert.doesNotMatch(out, /[<>]/, `quedan etiquetas en «${out}»`)
		assert.doesNotMatch(out, /evil\.example|alert|pw|enviar/, `queda contenido peligroso en «${out}»`)
		assert.ok(out.endsWith('ok'))
	}
})

test('trucos de anidado y codificación no se cuelan', () => {
	for (const evil of [
		'<scr<script>ipt>alert(1)</scr</script>ipt>',
		'<<script>script>alert(1)<</script>/script>',
		'<img src=x onerror=alert(1)//',
		'<a href="jav&#x61;script:alert(1)">x</a>',
		'<a href="&#106;avascript:alert(1)">x</a>',
		'<a href=javascript&colon;alert(1)>x</a>',
		'<img src="https://media.forgecdn.net/a.png"onerror="alert(1)">',
		'<p/onclick=alert(1)>x</p>',
		'<noscript><p title="</noscript><img src=x onerror=alert(1)>">',
		'<template><script>alert(1)</script></template>',
		'<base href="https://evil.example/">',
		'<meta http-equiv="refresh" content="0;url=https://evil.example">',
		'<link rel="stylesheet" href="https://evil.example/x.css">',
		'<div style="background:url(javascript:alert(1))">x</div>',
		'<table background="javascript:alert(1)"><td>x</td></table>',
	]) {
		assertSafe(evil)
	}
})

test('conserva el formato normal de una descripción', () => {
	const html = '<h2>Mapa</h2><p>Una <strong>aventura</strong> con <em>sorpresas</em>.</p><ul><li>Uno</li><li>Dos</li></ul><blockquote>cita</blockquote><table><tbody><tr><td>a</td><td>b</td></tr></tbody></table><hr><pre><code>/give @s apple</code></pre>'
	assert.equal(renderExternalHtml(html), html)
})

test('entradas raras no rompen nada y se acota el tamaño', () => {
	assert.equal(renderExternalHtml(''), '')
	assert.equal(renderExternalHtml(undefined as unknown as string), '')
	assert.equal(renderExternalHtml(null as unknown as string), '')
	assert.ok(renderExternalHtml('<p>x</p>'.repeat(100_000)).length <= 200_000 * 2)
})
